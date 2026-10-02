// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! **A decision put to a model, independent of who answers it** (eigenius#264 strand 2).
//!
//! Every call the reasoning layer makes to a model picks among described options: which structure
//! a sentence has, which sense a word takes. A [`Choice`] states that — what the decision is about,
//! and the [`Question`]s put about it, each with the facts it rests on and its options — and a
//! [`Decider`] answers every question of it together. The providers answer differently: Anthropic's
//! models write a reason and a ranking ([`AnthropicDecider`], through
//! [`super::anthropic_client`]); TypeSafe's System One model returns a calibrated probability per
//! option and no reason ([`TypeSafeDecider`]), several questions over one state in one request.
//! [`Decided`] carries whichever the provider gives.
//!
//! A choice is rendered per provider ([`render_prompt`], [`render_typesafe`]); both renderings are
//! pure and tested here, so what a model is asked can be read without calling one.

use std::collections::BTreeMap;

use serde_json::{json, Value};

/// One decision: what it is about, and the questions put about it, answered together.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Choice {
    /// What the questions are about, as named parts in order (the document, the earlier
    /// selections, the sentence). An empty part is left out.
    pub context: Vec<(String, String)>,
    pub questions: Vec<Question>,
}

/// One question of a [`Choice`]: what to decide, the facts it rests on, and the options.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Question {
    /// What to decide, with whatever the options' notation needs explained.
    pub question: String,
    /// Facts the decision rests on, as named lists (what the concepts mean). An empty list is left
    /// out.
    pub notes: Vec<(String, Vec<String>)>,
    /// The options, by key, each with what it means.
    pub options: Vec<(String, Description)>,
}

impl Choice {
    /// A choice of one question.
    pub fn single(context: Vec<(String, String)>, question: Question) -> Self {
        Choice {
            context,
            questions: vec![question],
        }
    }
}

/// What an option means: a text, or named fields in order (a grammatical analysis and the
/// functions of its phrases).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum Description {
    Text(String),
    Fields(Vec<(String, Field)>),
}

/// One field of a [`Description`].
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum Field {
    Text(String),
    List(Vec<String>),
}

impl From<&str> for Description {
    fn from(text: &str) -> Self {
        Description::Text(text.to_string())
    }
}

impl From<String> for Description {
    fn from(text: String) -> Self {
        Description::Text(text)
    }
}

/// A provider's answer to one [`Question`].
#[derive(Clone, Debug, PartialEq)]
pub struct Decided {
    /// The chosen option's key — one of the question's keys; [`Decider::choose`] checks it.
    pub choice: String,
    /// The other options, most plausible first, as far as the provider ranks them.
    pub runners_up: Vec<String>,
    /// The probability of each option, where the provider reports one.
    pub probabilities: BTreeMap<String, f64>,
    /// The provider's reason, where it gives one.
    pub rationale: String,
    /// The model that answered, as the provider names it (`jev-1.13.0`, `claude-sonnet-5-5`).
    pub model: String,
}

impl Decided {
    /// What the answer says beyond the choice: the reason, else the distribution.
    pub fn account(&self) -> String {
        if !self.rationale.is_empty() || self.probabilities.is_empty() {
            return self.rationale.clone();
        }
        let mut ps: Vec<(&String, &f64)> = self.probabilities.iter().collect();
        ps.sort_by(|a, b| b.1.total_cmp(a.1));
        let ps: Vec<String> = ps.iter().map(|(k, p)| format!("{k}={p:.2}")).collect();
        format!("probabilities {}", ps.join(", "))
    }

    /// How strongly the answer supports `key`: its probability where the provider gives one;
    /// otherwise 1 for the choice, a falling weight down the runners-up, and least for an option
    /// the answer leaves unranked. Comparable across one provider's answers, which is how a
    /// combination of answers is scored.
    pub fn weight(&self, key: &str) -> f64 {
        if !self.probabilities.is_empty() {
            return self.probabilities.get(key).copied().unwrap_or(0.0);
        }
        if key == self.choice {
            return 1.0;
        }
        match self.runners_up.iter().position(|r| r == key) {
            Some(i) => 0.5 / (i as f64 + 1.0),
            None => 0.01,
        }
    }
}

/// A model that answers [`Choice`]s.
pub trait Decider {
    /// Answer every question of `choice`, in order: `Err` on any transport, API or decode
    /// failure, or an answer naming no option of its question — the caller fails closed.
    fn choose(&self, choice: &Choice) -> Result<Vec<Decided>, String>;

    /// The model this decider asks.
    fn model(&self) -> &str;
}

impl<T: Decider + ?Sized> Decider for Box<T> {
    fn choose(&self, choice: &Choice) -> Result<Vec<Decided>, String> {
        (**self).choose(choice)
    }
    fn model(&self) -> &str {
        (**self).model()
    }
}

/// An answer is well-formed if it names an option of its question; its runners-up are kept only
/// as far as they name other options, each once.
pub fn checked(question: &Question, mut decided: Decided) -> Result<Decided, String> {
    let keys: Vec<&str> = question.options.iter().map(|(k, _)| k.as_str()).collect();
    if !keys.contains(&decided.choice.as_str()) {
        return Err(format!(
            "the answer names {:?}, which is not an option ({})",
            decided.choice,
            keys.join(", ")
        ));
    }
    let mut seen = vec![decided.choice.clone()];
    decided.runners_up.retain(|r| {
        let fresh = keys.contains(&r.as_str()) && !seen.contains(r);
        if fresh {
            seen.push(r.clone());
        }
        fresh
    });
    Ok(decided)
}

/// Every answer checked against its question; as many answers as questions.
pub fn checked_all(choice: &Choice, decided: Vec<Decided>) -> Result<Vec<Decided>, String> {
    if decided.len() != choice.questions.len() {
        return Err(format!(
            "{} answers for {} questions",
            decided.len(),
            choice.questions.len()
        ));
    }
    choice
        .questions
        .iter()
        .zip(decided)
        .map(|(q, d)| checked(q, d))
        .collect()
}

/// The id of question `n` (from 0) in a request and its reply: `q1`, `q2`, ….
pub fn question_id(n: usize) -> String {
    format!("q{}", n + 1)
}

/// The choice as one prompt, for a model that reads text and writes a reason.
pub fn render_prompt(choice: &Choice) -> String {
    let mut out = String::new();
    for (name, text) in choice.context.iter().filter(|(_, t)| !t.trim().is_empty()) {
        out.push_str(&format!("{name}:\n{}\n\n", text.trim_end()));
    }
    for (n, q) in choice.questions.iter().enumerate() {
        out.push_str(&format!("{}: {}\n", question_id(n), q.question.trim()));
        out.push_str("Options:\n");
        for (key, description) in &q.options {
            match description {
                Description::Text(text) => out.push_str(&format!("  [{key}] {text}\n")),
                Description::Fields(fields) => {
                    out.push_str(&format!("  [{key}]\n"));
                    for (name, field) in fields {
                        match field {
                            Field::Text(text) => out.push_str(&format!("      {name}: {text}\n")),
                            Field::List(items) => {
                                out.push_str(&format!("      {name}:\n"));
                                for item in items {
                                    out.push_str(&format!("        - {item}\n"));
                                }
                            }
                        }
                    }
                }
            }
        }
        for (name, lines) in q.notes.iter().filter(|(_, l)| !l.is_empty()) {
            out.push_str(&format!("{name}:\n"));
            for l in lines {
                out.push_str(&format!("  - {l}\n"));
            }
        }
        out.push('\n');
    }
    out.push_str(
        "For each question, return under its id `choice` = the key of the option that is right, \
         `rationale` = one or two sentences that decide it, and `runners_up` = the other keys, \
         most plausible first, less any its question excludes.",
    );
    out
}

/// The reply schema for [`render_prompt`]: one answer per question, under its id, the choice and
/// runners-up confined to that question's keys.
pub fn reply_schema(choice: &Choice) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for (n, q) in choice.questions.iter().enumerate() {
        let keys: Vec<&str> = q.options.iter().map(|(k, _)| k.as_str()).collect();
        properties.insert(
            question_id(n),
            json!({
                "type": "object",
                "properties": {
                    "choice": { "type": "string", "enum": keys },
                    "rationale": { "type": "string" },
                    "runners_up": { "type": "array", "items": { "type": "string", "enum": keys } },
                },
                "required": ["choice", "rationale", "runners_up"],
                "additionalProperties": false,
            }),
        );
        required.push(question_id(n));
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// The choice as a TypeSafe request body: the context as `state`, and each question as a `choice`
/// question under its id — its notes beside it in structured `instructions`, its options as
/// `criteria` — each in the choice's order. The model reads the JSON as text, and
/// `serde_json::Map` would sort the keys: the notes before the question, option `10` before `2`.
pub fn render_typesafe(choice: &Choice, model: &str) -> String {
    let text = |t: &str| Ordered::Value(Value::String(t.to_string()));
    let state = choice
        .context
        .iter()
        .filter(|(_, t)| !t.trim().is_empty())
        .map(|(name, t)| (snake(name), text(t.trim_end())))
        .collect();
    let questions = choice
        .questions
        .iter()
        .enumerate()
        .map(|(n, q)| {
            let instructions = std::iter::once(("question".to_string(), text(&q.question)))
                .chain(
                    q.notes
                        .iter()
                        .filter(|(_, l)| !l.is_empty())
                        .map(|(name, lines)| (snake(name), Ordered::Value(json!(lines)))),
                )
                .collect();
            let criteria = q
                .options
                .iter()
                .map(|(k, d)| {
                    let criterion = match d {
                        Description::Text(t) => text(t),
                        Description::Fields(fields) => Ordered::Object(
                            fields
                                .iter()
                                .map(|(name, field)| {
                                    let value = match field {
                                        Field::Text(t) => text(t),
                                        Field::List(items) => Ordered::Value(json!(items)),
                                    };
                                    (snake(name), value)
                                })
                                .collect(),
                        ),
                    };
                    (k.clone(), criterion)
                })
                .collect();
            (
                question_id(n),
                Ordered::Object(vec![
                    ("type".into(), text("choice")),
                    ("instructions".into(), Ordered::Object(instructions)),
                    ("criteria".into(), Ordered::Object(criteria)),
                ]),
            )
        })
        .collect();
    let body = Ordered::Object(vec![
        ("state".into(), Ordered::Object(state)),
        ("model".into(), text(model)),
        ("questions".into(), Ordered::Object(questions)),
    ]);
    serde_json::to_string(&body).expect("a JSON object of strings serializes")
}

/// JSON whose object keys keep the order they are given in.
enum Ordered {
    Value(Value),
    Object(Vec<(String, Ordered)>),
}

impl serde::Serialize for Ordered {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Ordered::Value(v) => v.serialize(s),
            Ordered::Object(fields) => {
                let mut m = s.serialize_map(Some(fields.len()))?;
                for (k, v) in fields {
                    m.serialize_entry(k, v)?;
                }
                m.end()
            }
        }
    }
}

/// A TypeSafe reply's `choice` answers as [`Decided`]s, for `questions` questions in order: the
/// runners-up are the other options by probability.
pub fn read_typesafe(payload: &Value, questions: usize) -> Result<Vec<Decided>, String> {
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    (0..questions)
        .map(|n| {
            let id = question_id(n);
            let answer = payload
                .get("answers")
                .and_then(|a| a.get(&id))
                .ok_or_else(|| format!("no `{id}` answer: {payload}"))?;
            let choice = answer
                .get("choice")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("no choice in the `{id}` answer: {payload}"))?
                .to_string();
            let probabilities: BTreeMap<String, f64> = answer
                .get("probabilities")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_f64().map(|p| (k.clone(), p)))
                        .collect()
                })
                .unwrap_or_default();
            let mut runners: Vec<(&String, &f64)> = probabilities
                .iter()
                .filter(|(k, _)| **k != choice)
                .collect();
            runners.sort_by(|a, b| b.1.total_cmp(a.1));
            Ok(Decided {
                runners_up: runners.into_iter().map(|(k, _)| k.clone()).collect(),
                rationale: String::new(),
                model: model.clone(),
                choice,
                probabilities,
            })
        })
        .collect()
}

/// An Anthropic reply as [`Decided`]s, for `choice`'s questions in order.
pub fn read_reply(reply: &Value, choice: &Choice, model: &str) -> Result<Vec<Decided>, String> {
    (0..choice.questions.len())
        .map(|n| {
            let id = question_id(n);
            let answer = reply
                .get(&id)
                .ok_or_else(|| format!("no `{id}` answer: {reply}"))?;
            let text = |k: &str| answer.get(k).and_then(Value::as_str).unwrap_or_default();
            Ok(Decided {
                choice: text("choice").to_string(),
                runners_up: answer
                    .get("runners_up")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                probabilities: BTreeMap::new(),
                rationale: text("rationale").to_string(),
                model: model.to_string(),
            })
        })
        .collect()
}

/// `How the structures differ` → `how_the_structures_differ`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

#[cfg(feature = "use-llm")]
mod providers {
    use super::*;
    use crate::dcg::anthropic_client::{anthropic_json, ModelConfig};

    /// Anthropic's models answer a choice with a reason, through the structured-output client.
    pub struct AnthropicDecider {
        api_key: String,
        cfg: ModelConfig,
    }

    impl AnthropicDecider {
        pub fn new(api_key: impl Into<String>, cfg: ModelConfig) -> Self {
            Self {
                api_key: api_key.into(),
                cfg,
            }
        }

        /// From `$ANTHROPIC_API_KEY`; `None` if unset.
        pub fn from_env(cfg: ModelConfig) -> Option<Self> {
            std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .map(|k| Self::new(k, cfg))
        }
    }

    /// Questions per request. The reply's schema is strict, and the API compiles it into a grammar
    /// it refuses past a size: a sense choice of 11 questions (`choice`, `rationale`, `runners_up`
    /// each) failed with "The compiled grammar is too large", and 10 compiled with 204 options
    /// (2026-10-01). The count of answer fields drives it, not the options.
    pub(crate) const STRICT_QUESTIONS: usize = 8;

    /// A choice asked in parts of at most [`STRICT_QUESTIONS`] questions, each with the whole
    /// context. The questions are independent, so the parts' answers concatenate in order.
    pub(crate) fn strict_parts(choice: &Choice) -> Vec<Choice> {
        choice
            .questions
            .chunks(STRICT_QUESTIONS)
            .map(|questions| Choice {
                context: choice.context.clone(),
                questions: questions.to_vec(),
            })
            .collect()
    }

    impl Decider for AnthropicDecider {
        fn choose(&self, choice: &Choice) -> Result<Vec<Decided>, String> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            let mut decided = Vec::with_capacity(choice.questions.len());
            for part in strict_parts(choice) {
                let reply = rt.block_on(anthropic_json(
                    &self.api_key,
                    &self.cfg,
                    &render_prompt(&part),
                    reply_schema(&part),
                ))?;
                decided.extend(checked_all(
                    &part,
                    read_reply(&reply, &part, &self.cfg.model)?,
                )?);
            }
            Ok(decided)
        }

        fn model(&self) -> &str {
            &self.cfg.model
        }
    }

    const TYPESAFE_URL: &str = "https://api.typesafe.ai/v1/systemone";

    /// Retries after a 429 (rate limit) or 529 (overloaded), with the delay doubling from this.
    const TYPESAFE_FIRST_BACKOFF_MS: u64 = 500;
    const TYPESAFE_TRIES: u32 = 6;

    /// TypeSafe's System One models answer a choice with a probability per option and no reason.
    pub struct TypeSafeDecider {
        api_key: String,
        model: String,
    }

    impl TypeSafeDecider {
        pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
            Self {
                api_key: api_key.into(),
                model: model.into(),
            }
        }

        /// From `$TYPESAFE_API_KEY`; `None` if unset.
        pub fn from_env(model: impl Into<String>) -> Option<Self> {
            std::env::var("TYPESAFE_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .map(|k| Self::new(k, model))
        }
    }

    impl Decider for TypeSafeDecider {
        fn choose(&self, choice: &Choice) -> Result<Vec<Decided>, String> {
            if let Some(q) = choice.questions.iter().find(|q| q.options.len() > 255) {
                return Err(format!(
                    "{} options; a TypeSafe choice takes at most 255",
                    q.options.len()
                ));
            }
            let body = render_typesafe(choice, &self.model);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            let payload = rt.block_on(async {
                let client = reqwest::Client::new();
                let mut wait = TYPESAFE_FIRST_BACKOFF_MS;
                for attempt in 1..=TYPESAFE_TRIES {
                    let resp = client
                        .post(TYPESAFE_URL)
                        .bearer_auth(&self.api_key)
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body.clone())
                        .send()
                        .await
                        .map_err(|e| format!("typesafe request failed: {e}"))?;
                    let status = resp.status();
                    let payload: Value = resp
                        .json()
                        .await
                        .map_err(|e| format!("typesafe response not JSON: {e}"))?;
                    if status.is_success() {
                        return Ok(payload);
                    }
                    let busy = matches!(status.as_u16(), 429 | 529);
                    if !busy || attempt == TYPESAFE_TRIES {
                        return Err(format!("typesafe API {status}: {payload}"));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                    wait *= 2;
                }
                unreachable!("the last attempt returns")
            })?;
            checked_all(choice, read_typesafe(&payload, choice.questions.len())?)
        }

        fn model(&self) -> &str {
            &self.model
        }
    }
}

#[cfg(feature = "use-llm")]
pub use providers::{AnthropicDecider, TypeSafeDecider};

/// The decider for `cfg`'s model, from the provider's key in the environment; `None` without one.
#[cfg(feature = "use-llm")]
pub fn decider_from_env(
    cfg: &crate::dcg::model_config::ModelConfig,
) -> Option<Box<dyn Decider + Send + Sync>> {
    use crate::dcg::model_config::Provider;
    match cfg.provider() {
        Provider::Anthropic => AnthropicDecider::from_env(cfg.clone())
            .map(|d| Box::new(d) as Box<dyn Decider + Send + Sync>),
        Provider::TypeSafe => TypeSafeDecider::from_env(cfg.model.clone())
            .map(|d| Box::new(d) as Box<dyn Decider + Send + Sync>),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question() -> Question {
        Question {
            question: "Which structure?".into(),
            notes: vec![(
                "How the structures differ".into(),
                vec!["«for X»: attaches to «a» in 1; to «b» in 2".into()],
            )],
            options: vec![
                ("1".into(), "structure one".into()),
                ("2".into(), "structure two".into()),
            ],
        }
    }

    fn choice() -> Choice {
        Choice::single(
            vec![
                ("Document".into(), "Doc text.".into()),
                ("Earlier selections".into(), String::new()),
                ("The sentence".into(), "S.".into()),
            ],
            question(),
        )
    }

    /// Two questions over one context: the senses of «analysed» and of «data».
    fn two_words() -> Choice {
        let q = |w: &str, a: &str, b: &str| Question {
            question: format!("Which sense of «{w}»?"),
            notes: Vec::new(),
            options: vec![("1".into(), a.into()), ("2".into(), b.into())],
        };
        Choice {
            context: vec![("the_sentence".into(), "We analysed data.".into())],
            questions: vec![
                q(
                    "analysed",
                    "consider in detail",
                    "break down into components",
                ),
                q("data", "an item of information", "a collection of facts"),
            ],
        }
    }

    /// The prompt names every option by key, carries the notes, and drops an empty context part.
    #[test]
    fn the_prompt_carries_the_options_and_notes() {
        let p = render_prompt(&choice());
        assert!(
            p.starts_with("Document:\nDoc text.\n\nThe sentence:\nS."),
            "{p}"
        );
        assert!(!p.contains("Earlier selections"), "{p}");
        assert!(
            p.contains(
                "q1: Which structure?\nOptions:\n  [1] structure one\n  [2] structure two\n"
            ),
            "{p}"
        );
        assert!(p.contains("How the structures differ:\n  - «for X»"), "{p}");
        let schema = reply_schema(&choice());
        assert_eq!(
            schema["properties"]["q1"]["properties"]["choice"]["enum"],
            json!(["1", "2"])
        );
    }

    /// The TypeSafe request puts the context in `state`, the notes beside the question, and the
    /// options in `criteria`; its answer's runners-up follow the probabilities.
    #[test]
    fn a_typesafe_request_and_answer_round_trip() {
        let req: Value = serde_json::from_str(&render_typesafe(&choice(), "jev-latest")).unwrap();
        assert_eq!(req["state"]["the_sentence"], json!("S."));
        assert!(req["state"].get("earlier_selections").is_none());
        let q = &req["questions"]["q1"];
        assert_eq!(q["type"], json!("choice"));
        assert_eq!(q["criteria"]["2"], json!("structure two"));
        assert_eq!(q["instructions"]["question"], json!("Which structure?"));
        assert!(q["instructions"]["how_the_structures_differ"].is_array());
        let payload = json!({
            "model": "jev-1.13.0",
            "answers": { "q1": {
                "type": "choice", "choice": "2", "confidence": 0.8,
                "probabilities": { "1": 0.1, "2": 0.9 },
            }},
        });
        let d = checked_all(&choice(), read_typesafe(&payload, 1).unwrap()).unwrap();
        assert_eq!(d[0].choice, "2");
        assert_eq!(d[0].runners_up, vec!["1".to_string()]);
        assert_eq!(d[0].model, "jev-1.13.0");
        assert_eq!(d[0].account(), "probabilities 2=0.90, 1=0.10");
        assert_eq!(d[0].weight("1"), 0.1);
    }

    /// Several questions go in one request, each under its id, and come back as one answer each.
    #[test]
    fn several_questions_are_asked_together() {
        let c = two_words();
        let req: Value = serde_json::from_str(&render_typesafe(&c, "jev-latest")).unwrap();
        assert_eq!(
            req["questions"]["q1"]["instructions"]["question"],
            json!("Which sense of «analysed»?")
        );
        assert_eq!(
            req["questions"]["q2"]["criteria"]["2"],
            json!("a collection of facts")
        );
        let payload = json!({
            "model": "jev-1.13.0",
            "answers": {
                "q1": { "type": "choice", "choice": "2", "probabilities": { "1": 0.3, "2": 0.7 } },
                "q2": { "type": "choice", "choice": "2", "probabilities": { "1": 0.2, "2": 0.8 } },
            },
        });
        let d = checked_all(&c, read_typesafe(&payload, 2).unwrap()).unwrap();
        assert_eq!((d[0].choice.as_str(), d[1].choice.as_str()), ("2", "2"));
        let schema = reply_schema(&c);
        assert_eq!(schema["required"], json!(["q1", "q2"]));
        let reply = json!({
            "q1": { "choice": "2", "rationale": "specimens", "runners_up": ["1"] },
            "q2": { "choice": "1", "rationale": "one item", "runners_up": [] },
        });
        let d = checked_all(&c, read_reply(&reply, &c, "claude").unwrap()).unwrap();
        assert_eq!(d[1].choice, "1");
        // Without probabilities, the choice outweighs a runner-up, which outweighs the unranked.
        assert!(d[0].weight("2") > d[0].weight("1") && d[0].weight("1") > d[1].weight("2"));
        assert!(
            checked_all(&c, d[..1].to_vec()).is_err(),
            "one answer for two questions"
        );
    }

    /// The body keeps the choice's order: the question before its notes, option 2 before 10.
    #[test]
    fn a_typesafe_request_keeps_the_choices_order() {
        let mut c = choice();
        c.questions[0].options = (1..=11)
            .map(|n| (n.to_string(), format!("s{n}").into()))
            .collect();
        let body = render_typesafe(&c, "jev-latest");
        let at = |needle: &str| body.find(needle).unwrap();
        assert!(at("\"document\"") < at("\"the_sentence\""));
        assert!(at("\"question\"") < at("\"how_the_structures_differ\""));
        assert!(at("\"2\":") < at("\"10\":"));
        assert!(at("\"state\"") < at("\"questions\""));
    }

    /// An option of named fields is an object of those fields for TypeSafe, in their order, and
    /// indented lines in the prompt.
    #[test]
    fn an_option_of_fields_renders_as_its_fields() {
        let mut c = choice();
        c.questions[0].options[0].1 = Description::Fields(vec![
            (
                "analysis".into(),
                Field::Text("We ascertained [MSI status] with sequencing.".into()),
            ),
            (
                "functions".into(),
                Field::List(vec![
                    "«with sequencing» is an adverbial of «ascertained»".into()
                ]),
            ),
        ]);
        let body = render_typesafe(&c, "jev-latest");
        let req: Value = serde_json::from_str(&body).unwrap();
        let one = &req["questions"]["q1"]["criteria"]["1"];
        assert_eq!(
            one["analysis"],
            json!("We ascertained [MSI status] with sequencing.")
        );
        assert_eq!(
            one["functions"][0],
            json!("«with sequencing» is an adverbial of «ascertained»")
        );
        assert!(body.find("\"analysis\"").unwrap() < body.find("\"functions\"").unwrap());
        let p = render_prompt(&c);
        assert!(p.contains("  [1]\n      analysis: We ascertained [MSI status] with sequencing.\n"));
        assert!(p.contains("      functions:\n        - «with sequencing» is an adverbial"));
    }

    /// An answer naming no option is an error; runners-up naming none are dropped.
    #[test]
    fn an_answer_outside_the_options_is_refused() {
        let d = |c: &str, r: &[&str]| Decided {
            choice: c.into(),
            runners_up: r.iter().map(|s| s.to_string()).collect(),
            probabilities: BTreeMap::new(),
            rationale: "because".into(),
            model: "m".into(),
        };
        assert!(checked(&question(), d("3", &[])).is_err());
        let ok = checked(&question(), d("1", &["9", "2", "2", "1"])).unwrap();
        assert_eq!(ok.runners_up, vec!["2".to_string()]);
    }

    /// A choice past the strict grammar's reach is asked in parts that keep the whole context and,
    /// concatenated, are the choice's questions in order.
    #[cfg(feature = "use-llm")]
    #[test]
    fn a_large_choice_is_asked_in_parts_that_keep_its_context_and_order() {
        use super::providers::{strict_parts, STRICT_QUESTIONS};
        let mut big = choice();
        big.questions = (0..2 * STRICT_QUESTIONS + 3)
            .map(|n| Question {
                question: format!("Which sense of word {n}?"),
                ..question()
            })
            .collect();
        let parts = strict_parts(&big);
        assert_eq!(
            parts.iter().map(|p| p.questions.len()).collect::<Vec<_>>(),
            vec![STRICT_QUESTIONS, STRICT_QUESTIONS, 3]
        );
        assert!(parts.iter().all(|p| p.context == big.context));
        let rejoined: Vec<Question> = parts.into_iter().flat_map(|p| p.questions).collect();
        assert_eq!(rejoined, big.questions);
    }
}

/// Live: each provider answers the witness of eigenius#264's strand 1 — an instrumental PP — through
/// its own transport. Skips without the provider's key; runs with `--features use-llm`.
#[cfg(all(test, feature = "use-llm"))]
mod live_tests {
    use super::*;
    use crate::dcg::model_config::ModelConfig;

    fn library() -> Choice {
        Choice::single(
            vec![(
                "The sentence".into(),
                "Project Achilles screened cell lines with a CRISPR library.".into(),
            )],
            Question {
                question: "Which structure does the sentence mean?".into(),
                notes: Vec::new(),
                options: vec![
                    (
                        "1".into(),
                        "the library is a property of the cell lines".into(),
                    ),
                    (
                        "2".into(),
                        "the library is the instrument of the screening".into(),
                    ),
                ],
            },
        )
    }

    fn ask(model: &str) {
        let Some(d) = decider_from_env(&ModelConfig::with_model(model)) else {
            eprintln!("SKIP {model}: no API key");
            return;
        };
        let answer = d
            .choose(&library())
            .expect("the provider answered")
            .remove(0);
        eprintln!(
            "{model}: {} — {} [{}]",
            answer.choice,
            answer.account(),
            answer.model
        );
        assert_eq!(answer.choice, "2", "the instrument");
        assert_eq!(answer.runners_up, vec!["1".to_string()]);
    }

    #[test]
    fn live_typesafe_decides_the_instrument() {
        ask("jev-latest");
    }

    #[test]
    fn live_claude5_decides_the_instrument() {
        ask("claude-sonnet-5-5");
    }

    #[test]
    fn live_claude4_decides_the_instrument() {
        ask("claude-sonnet-4-6");
    }
}
