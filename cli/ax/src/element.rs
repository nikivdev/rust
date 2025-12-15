//! Element reading and representation

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use colored::Colorize;

/// Bounding box for an element
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl BoundingBox {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x <= self.x + self.width &&
        y >= self.y && y <= self.y + self.height
    }
}

/// A UI element from accessibility API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub id: usize,
    pub role: String,
    pub label: String,
    pub bbox: BoundingBox,
    pub enabled: bool,
    pub focused: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub actions: Vec<String>,
    pub depth: usize,
}

impl Element {
    pub fn display(&self) -> String {
        let role_colored = match self.role.as_str() {
            "button" => self.role.blue(),
            "link" => self.role.cyan(),
            "textfield" => self.role.green(),
            "checkbox" | "radio" => self.role.yellow(),
            "menuitem" => self.role.magenta(),
            _ => self.role.white(),
        };

        let status = if self.focused {
            " [focused]".green()
        } else if !self.enabled {
            " [disabled]".dimmed()
        } else {
            "".normal()
        };

        format!(
            "{:>3} {} {:15} \"{}\"{}",
            self.id.to_string().dimmed(),
            "│".dimmed(),
            role_colored,
            self.label.white().bold(),
            status
        )
    }
}

/// Complete screen state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenState {
    pub elements: Vec<Element>,
    pub screen_width: i32,
    pub screen_height: i32,
    pub focused_app: String,
    pub timestamp: f64,
}

impl ScreenState {
    pub fn find_by_id(&self, id: usize) -> Option<&Element> {
        self.elements.iter().find(|e| e.id == id)
    }

    pub fn find_by_label(&self, label: &str) -> Vec<&Element> {
        let label_lower = label.to_lowercase();
        self.elements
            .iter()
            .filter(|e| e.label.to_lowercase().contains(&label_lower))
            .collect()
    }

    pub fn find_by_role(&self, role: &str) -> Vec<&Element> {
        let role_lower = role.to_lowercase();
        self.elements
            .iter()
            .filter(|e| e.role.to_lowercase() == role_lower)
            .collect()
    }

    pub fn element_at(&self, x: i32, y: i32) -> Option<&Element> {
        // Find smallest element containing the point
        self.elements
            .iter()
            .filter(|e| e.bbox.contains(x, y))
            .min_by_key(|e| e.bbox.width * e.bbox.height)
    }
}

/// Get current timestamp
fn timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Read accessibility elements from the focused application
#[cfg(target_os = "macos")]
pub fn read_screen_state(max_depth: usize) -> Result<ScreenState> {
    use accessibility::{AXUIElement, AXUIElementAttributes, AXAttribute, TreeWalker, TreeVisitor, TreeWalkerFlow};
    use core_foundation::base::{CFType, TCFType};
    use core_graphics::display::CGDisplay;
    use std::cell::RefCell;

    // Get screen size
    let main_display = CGDisplay::main();
    let screen_width = main_display.pixels_wide() as i32;
    let screen_height = main_display.pixels_high() as i32;

    // Get frontmost app name
    let focused_app_name = get_frontmost_app_name().unwrap_or_else(|| "Unknown".to_string());

    // Get elements from focused app
    let mut elements = Vec::new();
    let id_counter = RefCell::new(0usize);

    // Get frontmost application
    if let Some(app) = get_frontmost_app() {
        // Use TreeWalker for safe traversal
        struct ElementCollector<'a> {
            elements: &'a RefCell<Vec<Element>>,
            id_counter: &'a RefCell<usize>,
            max_depth: usize,
            current_depth: RefCell<usize>,
        }

        impl TreeVisitor for ElementCollector<'_> {
            fn enter_element(&self, element: &AXUIElement) -> TreeWalkerFlow {
                let depth = *self.current_depth.borrow();
                if depth > self.max_depth {
                    return TreeWalkerFlow::SkipSubtree;
                }

                // Get element properties
                let role = element.role()
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                let label = element.title()
                    .map(|s| s.to_string())
                    .or_else(|_| element.description().map(|s| s.to_string()))
                    .unwrap_or_default();

                let description = element.help().ok().map(|s| s.to_string());

                // Get actions
                let actions: Vec<String> = element.action_names()
                    .map(|arr| arr.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_default();

                // Get bounding box via position and size attributes
                let bbox = get_element_bbox(element);

                // Check enabled/focused
                let enabled = element.enabled()
                    .map(|b| b.into())
                    .unwrap_or(true);

                let focused = element.focused()
                    .map(|b| b.into())
                    .unwrap_or(false);

                // Get value for inputs
                let value = if role.contains("TextField") || role.contains("TextArea") {
                    element.attribute(&AXAttribute::value())
                        .ok()
                        .and_then(|v: CFType| {
                            // Try to convert CFType to string
                            use core_foundation::string::CFString;
                            unsafe {
                                if v.instance_of::<CFString>() {
                                    Some(CFString::wrap_under_get_rule(v.as_CFTypeRef() as _).to_string())
                                } else {
                                    None
                                }
                            }
                        })
                } else {
                    None
                };

                // Add element if it has content
                let is_interactive = is_interactive_role(&role);
                if is_interactive || !label.is_empty() {
                    let mut id = self.id_counter.borrow_mut();
                    self.elements.borrow_mut().push(Element {
                        id: *id,
                        role: simplify_role(&role),
                        label,
                        bbox,
                        enabled,
                        focused,
                        value,
                        description,
                        actions,
                        depth,
                    });
                    *id += 1;
                }

                *self.current_depth.borrow_mut() += 1;
                TreeWalkerFlow::Continue
            }

            fn exit_element(&self, _element: &AXUIElement) {
                let mut depth = self.current_depth.borrow_mut();
                if *depth > 0 {
                    *depth -= 1;
                }
            }
        }

        let elements_ref = RefCell::new(elements);
        let collector = ElementCollector {
            elements: &elements_ref,
            id_counter: &id_counter,
            max_depth,
            current_depth: RefCell::new(0),
        };

        let walker = TreeWalker::new();
        walker.walk(&app, &collector);

        elements = elements_ref.into_inner();
    }

    Ok(ScreenState {
        elements,
        screen_width,
        screen_height,
        focused_app: focused_app_name,
        timestamp: timestamp(),
    })
}

/// Get bounding box for an element
#[cfg(target_os = "macos")]
fn get_element_bbox(element: &accessibility::AXUIElement) -> BoundingBox {
    use accessibility::AXAttribute;
    use core_foundation::base::CFType;

    // Position attribute
    let position_attr = AXAttribute::<CFType>::new(
        &core_foundation::string::CFString::from_static_string("AXPosition")
    );
    // Size attribute
    let size_attr = AXAttribute::<CFType>::new(
        &core_foundation::string::CFString::from_static_string("AXSize")
    );

    let (x, y) = element.attribute(&position_attr)
        .ok()
        .and_then(|val| extract_point(&val))
        .unwrap_or((0, 0));

    let (width, height) = element.attribute(&size_attr)
        .ok()
        .and_then(|val| extract_size(&val))
        .unwrap_or((0, 0));

    BoundingBox { x, y, width, height }
}

/// Extract point from AXValue
#[cfg(target_os = "macos")]
fn extract_point(value: &core_foundation::base::CFType) -> Option<(i32, i32)> {
    use core_foundation::base::TCFType;
    use core_graphics::geometry::CGPoint;
    use accessibility_sys::AXValueGetValue;
    use std::mem::MaybeUninit;

    unsafe {
        let mut point = MaybeUninit::<CGPoint>::uninit();
        let success = AXValueGetValue(
            value.as_CFTypeRef() as _,
            1, // kAXValueCGPointType
            point.as_mut_ptr() as *mut _,
        );
        if success {
            let p = point.assume_init();
            Some((p.x as i32, p.y as i32))
        } else {
            None
        }
    }
}

/// Extract size from AXValue
#[cfg(target_os = "macos")]
fn extract_size(value: &core_foundation::base::CFType) -> Option<(i32, i32)> {
    use core_foundation::base::TCFType;
    use core_graphics::geometry::CGSize;
    use accessibility_sys::AXValueGetValue;
    use std::mem::MaybeUninit;

    unsafe {
        let mut size = MaybeUninit::<CGSize>::uninit();
        let success = AXValueGetValue(
            value.as_CFTypeRef() as _,
            2, // kAXValueCGSizeType
            size.as_mut_ptr() as *mut _,
        );
        if success {
            let s = size.assume_init();
            Some((s.width as i32, s.height as i32))
        } else {
            None
        }
    }
}

/// Get frontmost application as AXUIElement
#[cfg(target_os = "macos")]
fn get_frontmost_app() -> Option<accessibility::AXUIElement> {
    use cocoa::base::nil;
    use cocoa::foundation::NSAutoreleasePool;
    use objc::{msg_send, sel, sel_impl};

    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        // Get shared workspace
        let workspace: cocoa::base::id = msg_send![
            objc::class!(NSWorkspace),
            sharedWorkspace
        ];

        // Get frontmost application
        let frontmost: cocoa::base::id = msg_send![workspace, frontmostApplication];
        if frontmost == nil {
            return None;
        }

        // Get PID
        let pid: i32 = msg_send![frontmost, processIdentifier];

        Some(accessibility::AXUIElement::application(pid))
    }
}

/// Get name of frontmost application
#[cfg(target_os = "macos")]
fn get_frontmost_app_name() -> Option<String> {
    use cocoa::base::nil;
    use cocoa::foundation::NSAutoreleasePool;
    use objc::{msg_send, sel, sel_impl};

    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        let workspace: cocoa::base::id = msg_send![
            objc::class!(NSWorkspace),
            sharedWorkspace
        ];

        let frontmost: cocoa::base::id = msg_send![workspace, frontmostApplication];
        if frontmost == nil {
            return None;
        }

        let name: cocoa::base::id = msg_send![frontmost, localizedName];
        if name == nil {
            return None;
        }

        let bytes: *const i8 = msg_send![name, UTF8String];
        if bytes.is_null() {
            return None;
        }

        Some(std::ffi::CStr::from_ptr(bytes).to_string_lossy().into_owned())
    }
}

/// Check if role is interactive
fn is_interactive_role(role: &str) -> bool {
    matches!(
        role,
        "AXButton"
            | "AXLink"
            | "AXMenuItem"
            | "AXMenuBarItem"
            | "AXCheckBox"
            | "AXRadioButton"
            | "AXPopUpButton"
            | "AXComboBox"
            | "AXTextField"
            | "AXTextArea"
            | "AXTab"
            | "AXToolbarButton"
            | "AXDisclosureTriangle"
            | "AXIncrementor"
            | "AXSlider"
            | "AXColorWell"
            | "AXSearchField"
    )
}

/// Simplify macOS role names
fn simplify_role(role: &str) -> String {
    match role {
        "AXButton" | "AXToolbarButton" => "button",
        "AXLink" => "link",
        "AXMenuItem" | "AXMenuBarItem" => "menuitem",
        "AXCheckBox" => "checkbox",
        "AXRadioButton" => "radio",
        "AXPopUpButton" | "AXComboBox" => "dropdown",
        "AXTextField" | "AXSearchField" => "textfield",
        "AXTextArea" => "textarea",
        "AXTab" => "tab",
        "AXDisclosureTriangle" => "disclosure",
        "AXSlider" => "slider",
        "AXImage" => "image",
        "AXStaticText" => "text",
        "AXGroup" => "group",
        "AXWindow" => "window",
        "AXApplication" => "app",
        "AXScrollArea" => "scroll",
        "AXTable" | "AXOutline" => "table",
        "AXRow" => "row",
        "AXCell" => "cell",
        "AXColorWell" => "colorpicker",
        _ => role.strip_prefix("AX").unwrap_or(role),
    }
    .to_string()
}

// Fallback for non-macOS
#[cfg(not(target_os = "macos"))]
pub fn read_screen_state(_max_depth: usize) -> Result<ScreenState> {
    eprintln!("Warning: Not on macOS, returning synthetic data");
    Ok(ScreenState {
        elements: vec![
            Element {
                id: 0,
                role: "button".to_string(),
                label: "Submit".to_string(),
                bbox: BoundingBox { x: 100, y: 200, width: 80, height: 32 },
                enabled: true,
                focused: false,
                value: None,
                description: None,
                actions: vec!["AXPress".to_string()],
                depth: 0,
            },
        ],
        screen_width: 1920,
        screen_height: 1080,
        focused_app: "TestApp".to_string(),
        timestamp: timestamp(),
    })
}

// ============================================================================
// CLI Commands
// ============================================================================

pub fn list_elements(
    role: Option<String>,
    label: Option<String>,
    enabled_only: bool,
    depth: usize,
    json: bool,
) -> Result<()> {
    let state = read_screen_state(depth)?;

    let filtered: Vec<&Element> = state.elements.iter()
        .filter(|e| {
            if enabled_only && !e.enabled { return false; }
            if let Some(ref r) = role {
                if !e.role.to_lowercase().contains(&r.to_lowercase()) { return false; }
            }
            if let Some(ref l) = label {
                if !e.label.to_lowercase().contains(&l.to_lowercase()) { return false; }
            }
            true
        })
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
    } else {
        println!("{} elements in {}", filtered.len().to_string().green(), state.focused_app.cyan());
        println!("{}", "─".repeat(60).dimmed());
        for elem in filtered {
            println!("{}", elem.display());
        }
    }

    Ok(())
}

pub fn watch_elements(interval: u64, output: Option<String>, json: bool) -> Result<()> {
    println!("{}", "Watching accessibility tree (Ctrl+C to stop)...".yellow());

    loop {
        let state = read_screen_state(10)?;

        if json {
            let json_str = serde_json::to_string(&state)?;
            if let Some(ref path) = output {
                std::fs::write(path, &json_str)?;
            } else {
                println!("{}", json_str);
            }
        } else {
            // Clear screen and show update
            print!("\x1B[2J\x1B[1;1H");
            println!("{} │ {} elements", state.focused_app.cyan(), state.elements.len());
            println!("{}", "─".repeat(60).dimmed());
            for elem in &state.elements {
                println!("{}", elem.display());
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(interval));
    }
}

pub fn show_focused(json: bool) -> Result<()> {
    let state = read_screen_state(10)?;

    let focused = state.elements.iter().find(|e| e.focused);

    if json {
        println!("{}", serde_json::to_string_pretty(&focused)?);
    } else {
        match focused {
            Some(elem) => {
                println!("{}", "Focused element:".green());
                println!("{}", elem.display());
                if let Some(ref desc) = elem.description {
                    println!("  Description: {}", desc.dimmed());
                }
                if !elem.actions.is_empty() {
                    println!("  Actions: {}", elem.actions.join(", ").dimmed());
                }
            }
            None => {
                println!("{}", "No focused element".yellow());
            }
        }
    }

    Ok(())
}

pub fn element_at(x: i32, y: i32, json: bool) -> Result<()> {
    let state = read_screen_state(10)?;

    let element = state.element_at(x, y);

    if json {
        println!("{}", serde_json::to_string_pretty(&element)?);
    } else {
        match element {
            Some(elem) => {
                println!("{}", format!("Element at ({}, {}):", x, y).green());
                println!("{}", elem.display());
            }
            None => {
                println!("{}", format!("No element at ({}, {})", x, y).yellow());
            }
        }
    }

    Ok(())
}
