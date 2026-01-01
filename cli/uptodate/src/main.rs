use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use serde::{Deserialize, Serialize};

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

#[tokio::main]
async fn try_main() -> Result<()> {
    let cli = Cli::parse();

    // Parse GitHub username from URL or direct input
    let username = parse_github_username(&cli.input)?;

    // Calculate since date
    let since = if let Some(since_str) = &cli.since {
        parse_duration(since_str)?
    } else {
        Utc::now() - Duration::days(30) // Default: last 30 days
    };

    eprintln!("Fetching activity for @{} since {}", username, since.format("%Y-%m-%d"));

    // Fetch GitHub data
    let github_token = std::env::var("GITHUB_TOKEN").ok();
    let contact = fetch_github_contact(&username, since, github_token.as_deref()).await?;

    if cli.json {
        // Output JSON only
        println!("{}", serde_json::to_string_pretty(&contact)?);
    } else {
        // Display summary
        print_contact_summary(&contact);

        // Sync to linsa if requested
        if cli.sync {
            sync_to_linsa(&contact, &cli.api_url).await?;
        }

        // Save to local file
        if let Some(output) = &cli.output {
            let json = serde_json::to_string_pretty(&contact)?;
            std::fs::write(output, &json)?;
            println!("\nSaved to {}", output.display());
        } else {
            // Default: save to ~/.db/uptodate/<username>.json
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let dir = PathBuf::from(&home).join(".db").join("uptodate");
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{}.json", username));
            let json = serde_json::to_string_pretty(&contact)?;
            std::fs::write(&path, &json)?;
            println!("\nSaved to {}", path.display());
        }
    }

    Ok(())
}

#[derive(Parser)]
#[command(name = "uptodate", version, about = "Fetch GitHub user activity and store as Contact")]
struct Cli {
    /// GitHub URL or username (e.g., "steipete" or "https://github.com/steipete")
    input: String,

    /// Time range to fetch (e.g., "7d", "30d", "3m")
    #[arg(long)]
    since: Option<String>,

    /// Output JSON only (no file save)
    #[arg(long)]
    json: bool,

    /// Output file path (default: ~/.db/uptodate/<username>.json)
    #[arg(long, short)]
    output: Option<PathBuf>,

    /// Post to linsa API after fetching
    #[arg(long)]
    sync: bool,

    /// Linsa API URL (default: http://localhost:3000)
    #[arg(long, default_value = "http://localhost:3000")]
    api_url: String,
}

fn parse_github_username(input: &str) -> Result<String> {
    let input = input.trim();

    // Handle full URLs
    if input.starts_with("http://") || input.starts_with("https://") {
        let url = reqwest::Url::parse(input).context("Invalid URL")?;
        if url.host_str() != Some("github.com") {
            anyhow::bail!("URL must be a GitHub URL");
        }
        let path = url.path().trim_start_matches('/');
        let username = path.split('/').next().unwrap_or("");
        if username.is_empty() {
            anyhow::bail!("Could not extract username from URL");
        }
        Ok(username.to_string())
    } else {
        // Direct username
        Ok(input.to_string())
    }
}

fn parse_duration(s: &str) -> Result<DateTime<Utc>> {
    let s = s.trim().to_lowercase();
    let now = Utc::now();

    if let Some(days) = s.strip_suffix('d') {
        let n: i64 = days.parse().context("Invalid number of days")?;
        Ok(now - Duration::days(n))
    } else if let Some(weeks) = s.strip_suffix('w') {
        let n: i64 = weeks.parse().context("Invalid number of weeks")?;
        Ok(now - Duration::weeks(n))
    } else if let Some(months) = s.strip_suffix('m') {
        let n: i64 = months.parse().context("Invalid number of months")?;
        Ok(now - Duration::days(n * 30))
    } else {
        anyhow::bail!("Invalid duration format. Use: 7d, 2w, 3m")
    }
}

// === GitHub API Types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub name: String,
    pub username: String,
    pub platform: String,
    pub profile_url: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub blog: Option<String>,
    pub repos: u32,
    pub followers: u32,
    pub following: u32,
    pub recent_activity: Vec<GitHubActivity>,
    pub top_repos: Vec<RepoInfo>,
    pub last_fetched: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubActivity {
    #[serde(rename = "type")]
    pub activity_type: String,
    pub repo: String,
    pub title: String,
    pub url: String,
    pub date: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub url: String,
    pub stars: u32,
    pub forks: u32,
    pub language: Option<String>,
    pub updated_at: DateTime<Utc>,
}

// GitHub API response types
#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
    html_url: String,
    bio: Option<String>,
    company: Option<String>,
    location: Option<String>,
    blog: Option<String>,
    public_repos: u32,
    followers: u32,
    following: u32,
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    name: String,
    full_name: String,
    description: Option<String>,
    html_url: String,
    stargazers_count: u32,
    forks_count: u32,
    language: Option<String>,
    updated_at: DateTime<Utc>,
    fork: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubEvent {
    #[serde(rename = "type")]
    event_type: String,
    repo: GitHubEventRepo,
    created_at: DateTime<Utc>,
    payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct GitHubEventRepo {
    name: String,
}

async fn fetch_github_contact(
    username: &str,
    since: DateTime<Utc>,
    token: Option<&str>,
) -> Result<Contact> {
    let client = reqwest::Client::builder()
        .user_agent("uptodate-cli/0.1")
        .build()?;

    // Build headers with optional auth
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(token) = token {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", token).parse()?,
        );
    }

    // Fetch user profile
    eprint!("Fetching profile...");
    let user_url = format!("https://api.github.com/users/{}", username);
    let user: GitHubUser = client
        .get(&user_url)
        .headers(headers.clone())
        .send()
        .await?
        .error_for_status()
        .context("Failed to fetch user profile")?
        .json()
        .await?;
    eprintln!(" done");

    // Fetch repos (sorted by updated)
    eprint!("Fetching repos...");
    let repos_url = format!(
        "https://api.github.com/users/{}/repos?sort=updated&per_page=100",
        username
    );
    let repos: Vec<GitHubRepo> = client
        .get(&repos_url)
        .headers(headers.clone())
        .send()
        .await?
        .error_for_status()
        .context("Failed to fetch repos")?
        .json()
        .await?;
    eprintln!(" {} repos", repos.len());

    // Get top repos (non-forks, sorted by stars)
    let mut top_repos: Vec<RepoInfo> = repos
        .iter()
        .filter(|r| !r.fork)
        .map(|r| RepoInfo {
            name: r.name.clone(),
            full_name: r.full_name.clone(),
            description: r.description.clone(),
            url: r.html_url.clone(),
            stars: r.stargazers_count,
            forks: r.forks_count,
            language: r.language.clone(),
            updated_at: r.updated_at,
        })
        .collect();
    top_repos.sort_by(|a, b| b.stars.cmp(&a.stars));
    top_repos.truncate(10);

    // Fetch recent events
    eprint!("Fetching activity...");
    let events_url = format!(
        "https://api.github.com/users/{}/events?per_page=100",
        username
    );
    let events: Vec<GitHubEvent> = client
        .get(&events_url)
        .headers(headers.clone())
        .send()
        .await?
        .error_for_status()
        .context("Failed to fetch events")?
        .json()
        .await?;
    eprintln!(" {} events", events.len());

    // Convert events to activities
    let recent_activity: Vec<GitHubActivity> = events
        .into_iter()
        .filter(|e| e.created_at >= since)
        .filter_map(|e| event_to_activity(e))
        .collect();

    Ok(Contact {
        name: user.name.unwrap_or_else(|| user.login.clone()),
        username: user.login,
        platform: "github".to_string(),
        profile_url: user.html_url,
        avatar_url: user.avatar_url,
        bio: user.bio,
        company: user.company,
        location: user.location,
        blog: user.blog,
        repos: user.public_repos,
        followers: user.followers,
        following: user.following,
        recent_activity,
        top_repos,
        last_fetched: Utc::now(),
    })
}

fn event_to_activity(event: GitHubEvent) -> Option<GitHubActivity> {
    let (activity_type, title, url) = match event.event_type.as_str() {
        "PushEvent" => {
            let commits = event.payload.get("commits")?.as_array()?;
            let commit_count = commits.len();
            let msg = if commit_count == 1 {
                commits.first()?.get("message")?.as_str()?.lines().next()?.to_string()
            } else {
                format!("{} commits", commit_count)
            };
            (
                "commit".to_string(),
                msg,
                format!("https://github.com/{}", event.repo.name),
            )
        }
        "PullRequestEvent" => {
            let pr = event.payload.get("pull_request")?;
            let title = pr.get("title")?.as_str()?.to_string();
            let url = pr.get("html_url")?.as_str()?.to_string();
            ("pr".to_string(), title, url)
        }
        "IssuesEvent" => {
            let issue = event.payload.get("issue")?;
            let title = issue.get("title")?.as_str()?.to_string();
            let url = issue.get("html_url")?.as_str()?.to_string();
            ("issue".to_string(), title, url)
        }
        "WatchEvent" => {
            ("star".to_string(), format!("Starred {}", event.repo.name), format!("https://github.com/{}", event.repo.name))
        }
        "ForkEvent" => {
            ("fork".to_string(), format!("Forked {}", event.repo.name), format!("https://github.com/{}", event.repo.name))
        }
        "CreateEvent" => {
            let ref_type = event.payload.get("ref_type")?.as_str()?;
            if ref_type == "repository" {
                ("create".to_string(), format!("Created {}", event.repo.name), format!("https://github.com/{}", event.repo.name))
            } else {
                return None;
            }
        }
        _ => return None,
    };

    Some(GitHubActivity {
        activity_type,
        repo: event.repo.name,
        title,
        url,
        date: event.created_at,
    })
}

async fn sync_to_linsa(contact: &Contact, api_url: &str) -> Result<()> {
    // Get API key from environment
    let api_key = std::env::var("LINSA_API_KEY")
        .context("LINSA_API_KEY environment variable not set. Get one from linsa.io/settings")?;

    let client = reqwest::Client::new();
    let url = format!("{}/api/contacts", api_url.trim_end_matches('/'));

    // Build request body matching the API schema
    let body = serde_json::json!({
        "api_key": api_key,
        "name": contact.name,
        "username": contact.username,
        "platform": contact.platform,
        "profile_url": contact.profile_url,
        "avatar_url": contact.avatar_url,
        "bio": contact.bio,
        "company": contact.company,
        "location": contact.location,
        "blog": contact.blog,
        "repos": contact.repos,
        "followers": contact.followers,
        "following": contact.following,
        "recent_activity": contact.recent_activity.iter().map(|a| {
            serde_json::json!({
                "activity_type": a.activity_type,
                "repo": a.repo,
                "title": a.title,
                "url": a.url,
                "date": a.date.to_rfc3339(),
            })
        }).collect::<Vec<_>>(),
        "top_repos": contact.top_repos.iter().map(|r| {
            serde_json::json!({
                "name": r.name,
                "full_name": r.full_name,
                "description": r.description,
                "url": r.url,
                "stars": r.stars,
                "forks": r.forks,
                "language": r.language,
                "updated_at": r.updated_at.to_rfc3339(),
            })
        }).collect::<Vec<_>>(),
    });

    eprint!("Syncing to linsa...");
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("Failed to connect to linsa API")?;

    if response.status().is_success() {
        let result: serde_json::Value = response.json().await?;
        let action = result.get("action").and_then(|a| a.as_str()).unwrap_or("synced");
        eprintln!(" {} @{}", action, contact.username);
        Ok(())
    } else {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        anyhow::bail!("Linsa API error ({}): {}", status, error_text)
    }
}

fn print_contact_summary(contact: &Contact) {
    println!("\n{} (@{})", contact.name, contact.username);
    println!("{}", "=".repeat(40));

    if let Some(bio) = &contact.bio {
        println!("{}", bio);
    }

    println!("\nStats: {} repos | {} followers | {} following",
        contact.repos, contact.followers, contact.following);

    if let Some(company) = &contact.company {
        println!("Company: {}", company);
    }
    if let Some(location) = &contact.location {
        println!("Location: {}", location);
    }

    if !contact.top_repos.is_empty() {
        println!("\nTop Repos:");
        for repo in contact.top_repos.iter().take(5) {
            let lang = repo.language.as_deref().unwrap_or("?");
            println!("  {} ({}) - {} stars", repo.name, lang, repo.stars);
        }
    }

    if !contact.recent_activity.is_empty() {
        println!("\nRecent Activity ({} events):", contact.recent_activity.len());
        for activity in contact.recent_activity.iter().take(10) {
            let date = activity.date.format("%m/%d");
            println!("  [{}] {} - {}", date, activity.activity_type, activity.title);
        }
    }
}
