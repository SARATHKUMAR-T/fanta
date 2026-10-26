//! Teacher mode: turns a topic into a short visual lesson - a big-picture
//! diagram, an analogy, a few steps (each with its own diagram), key terms and a
//! quiz. The model describes diagrams as nodes and edges rather than writing
//! diagram code, so a small local model can't produce one that fails to render.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::llm::Message;
use crate::{brain, db, AppState};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Node {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Diagram {
    /// "flow" (a process), "sequence" (who talks to whom, in order),
    /// "tree" (parts of a whole), or "cycle" (a loop that repeats).
    pub kind: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Step {
    pub title: String,
    pub explain: String,
    pub example: String,
    pub key_point: String,
    pub diagram: Diagram,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Term {
    pub term: String,
    pub meaning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Question {
    pub question: String,
    pub options: Vec<String>,
    pub answer: usize,
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Lesson {
    pub id: i64,
    pub topic: String,
    pub level: String,
    pub title: String,
    pub summary: String,
    pub analogy: String,
    pub overview: Diagram,
    pub steps: Vec<Step>,
    pub terms: Vec<Term>,
    pub quiz: Vec<Question>,
}

/// An answer to a follow-up question about a lesson.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Answer {
    pub question: String,
    pub answer: String,
    pub diagram: Diagram,
}

#[derive(Debug, Clone, Serialize)]
pub struct LessonInfo {
    pub id: i64,
    pub topic: String,
    pub title: String,
    pub level: String,
    pub created_at: String,
}

pub const LEVELS: [&str; 3] = ["kid", "beginner", "intermediate"];

fn level_guide(level: &str) -> &'static str {
    match level {
        "kid" => "Explain it to a curious 10-year-old: everyday words only, short sentences, playful analogies.",
        "intermediate" => "The learner knows the basics: use the real terms (defined once), cover how and why, mention one common pitfall.",
        _ => "The learner is new to this: plain words, define any jargon the first time, one idea per step.",
    }
}

const TEACHER_PROMPT: &str = "You are a patient, visual teacher. You explain things so they click: \
start with the big picture, use one vivid everyday analogy, then walk through 3 to 5 small steps. \
Every step gets a diagram. Keep text short - the diagrams carry the structure.

Diagrams are nodes and edges:
- kind \"flow\": a process from start to end. \"sequence\": parties exchanging messages in order \
(nodes are the parties; edges are the messages, in order). \"tree\": a whole and its parts \
(edges go parent -> child, one root). \"cycle\": steps that repeat (the last edge returns to the first node).
- 3 to 7 nodes. Node ids are short (a, b, c...). Labels are 1-5 words. Edge labels are 0-4 words.

Text rules: explain is 2-3 short sentences. example is one concrete real-world example. \
key_point is the one thing to remember, under 12 words. summary is 1-2 sentences. \
terms: 3-5 key words with plain meanings. quiz: 3 multiple-choice questions with 3-4 options; \
answer is the 0-based index of the correct option; why explains it in one sentence. \
Questions check understanding, not memory of wording.";

fn diagram_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": { "type": "string", "enum": ["flow", "sequence", "tree", "cycle"] },
            "nodes": { "type": "array", "items": {
                "type": "object",
                "properties": { "id": { "type": "string" }, "label": { "type": "string" } },
                "required": ["id", "label"]
            }},
            "edges": { "type": "array", "items": {
                "type": "object",
                "properties": { "from": { "type": "string" }, "to": { "type": "string" }, "label": { "type": "string" } },
                "required": ["from", "to", "label"]
            }}
        },
        "required": ["kind", "nodes", "edges"]
    })
}

fn lesson_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "summary": { "type": "string" },
            "analogy": { "type": "string" },
            "overview": diagram_schema(),
            "steps": { "type": "array", "items": {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "explain": { "type": "string" },
                    "example": { "type": "string" },
                    "key_point": { "type": "string" },
                    "diagram": diagram_schema()
                },
                "required": ["title", "explain", "example", "key_point", "diagram"]
            }},
            "terms": { "type": "array", "items": {
                "type": "object",
                "properties": { "term": { "type": "string" }, "meaning": { "type": "string" } },
                "required": ["term", "meaning"]
            }},
            "quiz": { "type": "array", "items": {
                "type": "object",
                "properties": {
                    "question": { "type": "string" },
                    "options": { "type": "array", "items": { "type": "string" } },
                    "answer": { "type": "integer" },
                    "why": { "type": "string" }
                },
                "required": ["question", "options", "answer", "why"]
            }}
        },
        "required": ["title", "summary", "analogy", "overview", "steps", "terms", "quiz"]
    })
}

fn answer_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "answer": { "type": "string" }, "diagram": diagram_schema() },
        "required": ["answer", "diagram"]
    })
}

/// The JSON object in a reply that may wrap it in prose or a ```json fence.
fn extract_json(text: &str) -> anyhow::Result<Value> {
    let start = text.find('{').ok_or_else(|| anyhow::anyhow!("the model didn't return a lesson"))?;
    let end = text.rfind('}').ok_or_else(|| anyhow::anyhow!("the model's lesson was cut off"))?;
    Ok(serde_json::from_str(&text[start..=end])?)
}

/// Asks the user's chosen model for JSON matching `schema`. The local model is
/// constrained to the schema; cloud models are told it and their reply parsed.
async fn ask_json(app: &AppHandle, system: &str, user: &str, schema: &Value) -> anyhow::Result<Value> {
    let state = app.state::<AppState>();
    if !brain::is_cloud(app) {
        let messages = [Message::new("system", system), Message::new("user", user)];
        return state.ollama.complete_json(&state.persona.model, &messages, schema).await;
    }
    let system = format!("{system}\n\nReply with only a JSON object matching this JSON schema, no prose:\n{schema}");
    let messages = [Message::new("system", system), Message::new("user", user)];
    let mut chars = 0usize;
    let reply = brain::chat(app, &messages, &[], |t| {
        chars += t.len();
        // Roughly how far along it is, for the progress bar (a lesson is ~6k characters).
        let _ = app.emit("teach://progress", (chars as f64 / 6000.0).min(0.95));
    })
    .await?;
    extract_json(&reply.content)
}

/// Fixes what models commonly get wrong, so the UI can trust the shape.
fn tidy(lesson: &mut Lesson) {
    lesson.steps.retain(|s| !s.title.trim().is_empty() || !s.explain.trim().is_empty());
    for s in &mut lesson.steps {
        tidy_diagram(&mut s.diagram);
    }
    tidy_diagram(&mut lesson.overview);
    lesson.terms.retain(|t| !t.term.trim().is_empty());
    lesson.quiz.retain(|q| q.options.len() >= 2 && q.answer < q.options.len() && !q.question.trim().is_empty());
}

fn tidy_diagram(d: &mut Diagram) {
    if !["flow", "sequence", "tree", "cycle"].contains(&d.kind.as_str()) {
        d.kind = "flow".into();
    }
    d.nodes.retain(|n| !n.id.trim().is_empty());
    let mut seen = std::collections::HashSet::new();
    d.nodes.retain(|n| seen.insert(n.id.clone()));
    for n in &mut d.nodes {
        if n.label.trim().is_empty() {
            n.label = n.id.clone();
        }
    }
    // An edge to a node the model forgot to list: add it, labelled with its id.
    for e in d.edges.clone() {
        for id in [&e.from, &e.to] {
            if !id.trim().is_empty() && !d.nodes.iter().any(|n| &n.id == id) {
                d.nodes.push(Node { id: id.clone(), label: id.clone() });
            }
        }
    }
    d.edges.retain(|e| !e.from.trim().is_empty() && !e.to.trim().is_empty());
}

pub async fn lesson(app: &AppHandle, topic: &str, level: &str) -> anyhow::Result<Lesson> {
    let topic = topic.trim();
    if topic.is_empty() {
        anyhow::bail!("tell me what you'd like to learn");
    }
    let level = if LEVELS.contains(&level) { level } else { "beginner" };
    let system = format!("{TEACHER_PROMPT}\n\nLevel: {}", level_guide(level));
    let value = ask_json(app, &system, &format!("Teach me: {topic}"), &lesson_schema()).await?;
    let mut lesson: Lesson = serde_json::from_value(value)?;
    tidy(&mut lesson);
    if lesson.steps.is_empty() {
        anyhow::bail!("the model's lesson had no steps - try again, or phrase the topic differently");
    }
    lesson.topic = topic.to_string();
    lesson.level = level.to_string();
    let state = app.state::<AppState>();
    lesson.id =
        db::insert_lesson(&state.db.lock().unwrap(), topic, &lesson.title, level, &serde_json::to_string(&lesson)?)?;
    Ok(lesson)
}

/// Answers a follow-up question in the context of a lesson, with a diagram.
pub async fn ask(app: &AppHandle, lesson_id: i64, question: &str) -> anyhow::Result<Answer> {
    let question = question.trim();
    if question.is_empty() {
        anyhow::bail!("ask me something about the lesson");
    }
    let lesson = get(app, lesson_id)?;
    let outline = lesson
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {}: {}", i + 1, s.title, s.explain))
        .collect::<Vec<_>>()
        .join("\n");
    let system = format!(
        "{TEACHER_PROMPT}\n\nLevel: {}\n\nYou already taught this lesson:\n{}\n{}\n{outline}\n\n\
         Now answer the learner's follow-up in 2-4 short sentences, building on the lesson, \
         and draw one diagram that makes the answer clear.",
        level_guide(&lesson.level),
        lesson.title,
        lesson.summary
    );
    let value = ask_json(app, &system, question, &answer_schema()).await?;
    let mut answer: Answer = serde_json::from_value(value)?;
    tidy_diagram(&mut answer.diagram);
    answer.question = question.to_string();
    Ok(answer)
}

pub fn get(app: &AppHandle, id: i64) -> anyhow::Result<Lesson> {
    let json = db::get_lesson(&app.state::<AppState>().db.lock().unwrap(), id)?
        .ok_or_else(|| anyhow::anyhow!("that lesson is gone"))?;
    let mut lesson: Lesson = serde_json::from_str(&json)?;
    lesson.id = id;
    Ok(lesson)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_json_in_prose() {
        let v = extract_json("Sure! ```json\n{\"title\": \"DNS\"}\n```").unwrap();
        assert_eq!(v["title"], "DNS");
        assert!(extract_json("no lesson here").is_err());
    }

    #[test]
    fn tidies_diagrams() {
        let mut d = Diagram {
            kind: "mindmap".into(),
            nodes: vec![
                Node { id: "a".into(), label: "Browser".into() },
                Node { id: "a".into(), label: "dup".into() },
                Node { id: "b".into(), label: "".into() },
            ],
            edges: vec![
                Edge { from: "a".into(), to: "b".into(), label: "asks".into() },
                Edge { from: "b".into(), to: "c".into(), label: "".into() },
                Edge { from: "".into(), to: "a".into(), label: "".into() },
            ],
        };
        tidy_diagram(&mut d);
        assert_eq!(d.kind, "flow");
        let labels: Vec<_> = d.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["Browser", "b", "c"]);
        assert_eq!(d.edges.len(), 2);
    }

    #[test]
    fn drops_broken_quiz_questions() {
        let mut l = Lesson {
            steps: vec![Step { title: "One".into(), ..Default::default() }],
            quiz: vec![
                Question {
                    question: "Ok?".into(),
                    options: vec!["a".into(), "b".into()],
                    answer: 1,
                    why: String::new(),
                },
                Question {
                    question: "Bad index".into(),
                    options: vec!["a".into(), "b".into()],
                    answer: 2,
                    why: String::new(),
                },
                Question { question: "One option".into(), options: vec!["a".into()], answer: 0, why: String::new() },
            ],
            ..Default::default()
        };
        tidy(&mut l);
        assert_eq!(l.quiz.len(), 1);
    }
}
