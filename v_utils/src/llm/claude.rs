use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::llm::{Conversation, LlmConversation, LlmResponse, Model, Response, Role};

#[allow(dead_code)]
#[derive(Debug)]
enum ClaudeModel {
	Haiku,
	Sonnet,
	Opus,
}
pub struct Cost {
	pub million_input_tokens: f32,
	pub million_output_tokens: f32,
}

impl ClaudeModel {
	fn to_str(&self) -> &str {
		match self {
			ClaudeModel::Haiku => "claude-3-haiku-20240307",
			ClaudeModel::Sonnet => "claude-3-sonnet-20240229",
			ClaudeModel::Opus => "claude-3-opus-20240229",
		}
	}

	fn from_str(s: &str) -> Self {
		match s {
			_ if s.to_lowercase().contains("haiku") => Self::Haiku,
			_ if s.to_lowercase().contains("sonnet") => Self::Sonnet,
			_ if s.to_lowercase().contains("opus") => Self::Opus,
			_ => panic!("Unknown model: {}", s),
		}
	}

	pub fn cost(&self) -> Cost {
		match self {
			Self::Haiku => Cost {
				million_input_tokens: 0.25,
				million_output_tokens: 1.25,
			},
			Self::Sonnet => Cost {
				million_input_tokens: 3.0,
				million_output_tokens: 15.0,
			},
			Self::Opus => Cost {
				million_input_tokens: 15.0,
				million_output_tokens: 75.0,
			},
		}
	}

	pub fn from_general(model: Model) -> Self {
		match model {
			Model::Fast => Self::Haiku,
			Model::Medium => Self::Sonnet,
			Model::Slow => Self::Opus,
		}
	}
}

#[derive(Debug, Serialize)]
struct ClaudeMessage {
	role: String,
	content: String,
}
#[derive(Debug, Serialize)]
struct ClaudeConversation {
	messages: Vec<ClaudeMessage>,
}
impl LlmConversation for ClaudeConversation {
	fn new(conversation: &Conversation) -> Self {
		let mut messages = Vec::new();
		for message in &conversation.0 {
			messages.push(ClaudeMessage {
				role: {
					match message.role {
						Role::System => "system".to_string(),
						Role::User => "user".to_string(),
						Role::Assistant => "assistant".to_string(),
					}
				},
				content: message.content.to_string(),
			});
		}
		Self { messages }
	}
}

///docs: https://docs.anthropic.com/claude/reference/messages_post
pub fn ask_claude<T: AsRef<str>>(
	conversation: &Conversation,
	model: Model,
	max_tokens: Option<usize>,
	stop_sequences: Option<Vec<T>>,
) -> Result<Response> {
	let mut conversation = ClaudeConversation::new(&conversation);

	let api_key = std::env::var("CLAUDE_TOKEN").expect("CLAUDE_TOKEN environment variable not set");
	let url = "https://api.anthropic.com/v1/messages";

	let mut headers = HeaderMap::new();
	headers.insert("x-api-key", HeaderValue::from_str(&api_key).unwrap());
	headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
	headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

	let system_message = match conversation.messages[0].role == "system" {
		true => {
			let system_message = conversation.messages.remove(0);
			Some(system_message.content)
		}
		false => None,
	};

	let max_tokens = match max_tokens {
		Some(max) => max,
		None => 4096, // on 3rd generaltion of claude, this is maximum output size for every one of the models
	};

	let mut payload = json!({
		"model": ClaudeModel::from_general(model.clone()).to_str(),
		"temperature": 0.0,
		"max_tokens": max_tokens,
		"messages": conversation.messages
	});
	if let Some(stop_seqs) = stop_sequences {
		let stop_seqs_str: Vec<String> = stop_seqs.into_iter().map(|s| s.as_ref().to_string()).collect();
		payload
			.as_object_mut()
			.unwrap()
			.insert("stop_sequences".to_string(), serde_json::json!(stop_seqs_str));
	}
	if let Some(system_message) = system_message {
		payload.as_object_mut().unwrap().insert("system".to_string(), serde_json::json!(system_message));
	}

	let client = Client::new();
	let response = client.post(url).headers(headers).json(&payload).send()?;

	let response_raw = response.text()?;
	let response: ClaudeResponse =
		serde_json::from_str(&response_raw).map_err(|_| anyhow!("Failed to read response from anthropic api: {}", &response_raw))?;
	Ok(response.to_general_form())
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct ClaudeResponse {
	id: String,
	#[serde(rename = "type")]
	response_type: String,
	role: String,
	content: Vec<ClaudeContent>,
	model: String,
	stop_reason: String,
	stop_sequence: Option<String>,
	usage: ClaudeUsage,
}

impl ClaudeResponse {
	pub fn text(&self) -> String {
		self.content[0].text.clone()
	}

	pub fn cost_cents(&self) -> f32 {
		let model = ClaudeModel::from_str(&self.model);
		let cost = model.cost();
		(self.usage.input_tokens as f32 * cost.million_input_tokens + self.usage.output_tokens as f32 * cost.million_output_tokens) / 10_000.0
	}
}

impl LlmResponse for ClaudeResponse {
	fn to_general_form(&self) -> Response {
		Response {
			text: self.text(),
			cost_cents: self.cost_cents(),
		}
	}
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ClaudeContent {
	#[serde(rename = "type")]
	content_type: String,
	text: String,
}
#[derive(Deserialize, Debug)]
struct ClaudeUsage {
	input_tokens: u32,
	output_tokens: u32,
}
