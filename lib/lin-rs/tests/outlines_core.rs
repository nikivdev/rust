use outlines_core::prelude::*;
use serde::Deserialize;

const TOOL_CALL_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "tool": { "type": "string", "enum": ["search", "weather_lookup"] },
    "arguments": {
      "type": "object",
      "properties": {
        "query": { "type": "string" },
        "location": { "type": "string" }
      },
      "required": ["query"]
    }
  },
  "required": ["tool", "arguments"]
}"#;

#[derive(Debug, Deserialize, PartialEq)]
struct ToolCall {
    tool: String,
    arguments: ToolArguments,
}

#[derive(Debug, Deserialize, PartialEq)]
struct ToolArguments {
    query: String,
    location: Option<String>,
}

#[test]
fn structured_tool_call_flow() -> Result<(), outlines_core::Error> {
    // Schema -> regex
    let regex = json_schema::regex_from_str(TOOL_CALL_SCHEMA, None, Some(3))?;

    // Minimal ASCII vocabulary that can emit the sample response.
    let vocabulary = build_ascii_vocabulary();
    let index = Index::new(&regex, &vocabulary)?;

    // Ensure the guide has somewhere to start.
    let initial_state = index.initial_state();
    let allowed = index.allowed_tokens(&initial_state).expect("allowed tokens");
    assert!(!allowed.is_empty(), "initial DFA state should permit tokens");

    // Pretend this came from an LLM.
    let response = r#"{"tool":"search","arguments":{"query":"rust-lang news","location":"world"}}"#;

    // Map the response string into token ids and walk the DFA to ensure it is accepted.
    let mut state = initial_state;
    for token_id in encode_ascii(response, &vocabulary) {
        state = index
            .next_state(&state, &token_id)
            .expect("transition should exist for guided output");
    }
    assert!(
        index.is_final_state(&state),
        "LLM output should land in a final DFA state"
    );

    // Parse to a strongly typed tool call for downstream dispatch.
    let parsed: ToolCall = serde_json::from_str(response).expect("LLM output should be valid JSON");
    assert_eq!(
        parsed,
        ToolCall {
            tool: "search".to_string(),
            arguments: ToolArguments {
                query: "rust-lang news".to_string(),
                location: Some("world".to_string()),
            }
        }
    );

    Ok(())
}

fn build_ascii_vocabulary() -> Vocabulary {
    // Use a high EOS id to keep single-byte tokens distinct.
    let eos_token_id = u32::MAX;
    let mut vocabulary = Vocabulary::new(eos_token_id);

    // Cover the printable ASCII range so our sample JSON can always be expressed.
    for (idx, byte) in (b' '..=b'~').enumerate() {
        vocabulary
            .try_insert(vec![byte], idx as TokenId)
            .expect("token should insert");
    }

    vocabulary
}

fn encode_ascii(input: &str, vocabulary: &Vocabulary) -> Vec<TokenId> {
    input
        .bytes()
        .map(|byte| {
            *vocabulary
                .token_ids(&[byte])
                .and_then(|ids| ids.first())
                .expect("token id for byte should exist")
        })
        .collect()
}
