//! The list of tools: each tool's name, description, parameter schema and code.
//!
//! A tool is declared once with [`declare_tool`]: its parameters are a Rust
//! struct deriving `Deserialize` (to read Claude's arguments) and `JsonSchema`
//! (to describe them). The JSON schema is generated from that struct, field
//! documentation included, so the schema Claude reads can never drift from the
//! code that reads the arguments.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::output::{ToolError, ToolOutput};
use crate::toolbox::ToolCall;

/// What a tool may do, for clients that show or confirm tool calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEffect {
    /// Only reads the project or files.
    ReadsOnly,
    /// Changes the open project (undoable) and saves it.
    ChangesProject,
    /// Creates files on disk (a new project, an export), never replacing one
    /// without `"overwrite": true`.
    WritesFiles,
}

/// The description of one tool, as shown to Claude.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    /// Unique name, for example `"split_clip"`.
    pub name: &'static str,
    /// Short human title, for example `"Split a clip"`.
    pub title: &'static str,
    /// What the tool does, written for a language model: units, coordinate
    /// system, examples.
    pub description: &'static str,
    /// JSON schema (draft 2020-12) of the arguments: always an object.
    pub input_schema: Map<String, Value>,
    /// What the tool may change.
    pub effect: ToolEffect,
}

/// The function running a tool, from its raw JSON arguments.
type ToolRunner =
    Box<dyn Fn(&mut ToolCall<'_>, Value) -> Result<ToolOutput, ToolError> + Send + Sync>;

/// A tool: its definition and the code that runs it.
pub(crate) struct ToolEntry {
    pub(crate) definition: ToolDefinition,
    pub(crate) run: ToolRunner,
}

/// The parameters of a tool that takes none.
#[derive(Debug, serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoParameters {}

/// Declares a tool whose arguments are read into `Parameters`.
pub(crate) fn declare_tool<Parameters>(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    effect: ToolEffect,
    run: fn(&mut ToolCall<'_>, Parameters) -> Result<ToolOutput, ToolError>,
) -> ToolEntry
where
    Parameters: DeserializeOwned + JsonSchema + 'static,
{
    ToolEntry {
        definition: ToolDefinition {
            name,
            title,
            description,
            input_schema: input_schema_for::<Parameters>(),
            effect,
        },
        run: Box::new(move |toolbox, arguments| {
            let parameters = read_arguments::<Parameters>(name, arguments)?;
            run(toolbox, parameters)
        }),
    }
}

/// Reads Claude's arguments, explaining any mistake (missing, unknown or
/// mistyped field).
fn read_arguments<Parameters: DeserializeOwned>(
    tool_name: &str,
    arguments: Value,
) -> Result<Parameters, ToolError> {
    let arguments = match arguments {
        Value::Null => Value::Object(Map::new()),
        other => other,
    };
    serde_json::from_value(arguments).map_err(|error| {
        ToolError::new(format!(
            "the arguments of {tool_name} are not valid: {error}. Check the names, types and \
             units against the tool's input schema"
        ))
    })
}

/// The JSON schema of `Parameters`, with every sub-schema written in place
/// (no `$ref`), which language models read more reliably.
fn input_schema_for<Parameters: JsonSchema>() -> Map<String, Value> {
    let generator = SchemaSettings::draft2020_12()
        .with(|settings| settings.inline_subschemas = true)
        .into_generator();
    let schema = generator.into_root_schema_for::<Parameters>();
    let mut schema_object = match schema.to_value() {
        Value::Object(object) => object,
        _ => Map::new(),
    };
    // The schema dialect, the Rust struct name and the struct's documentation
    // (written for developers) mean nothing to Claude: the tool description
    // says what the tool does.
    schema_object.remove("$schema");
    schema_object.remove("title");
    schema_object.remove("description");
    schema_object
        .entry("type")
        .or_insert_with(|| Value::String("object".to_string()));
    schema_object
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()));
    let mut schema_value = Value::Object(schema_object);
    join_wrapped_description_lines(&mut schema_value);
    match schema_value {
        Value::Object(object) => object,
        _ => Map::new(),
    }
}

/// Field descriptions come from doc comments wrapped at 100 columns: their line
/// breaks are not meant as such, so they become spaces.
fn join_wrapped_description_lines(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                match child {
                    Value::String(text) if key == "description" => *text = text.replace('\n', " "),
                    _ => join_wrapped_description_lines(child),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(join_wrapped_description_lines),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parameters of a pretend tool.
    #[derive(Debug, serde::Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct ExampleParameters {
        /// The clip to cut, for example 4.
        clip_id: u64,
        /// Where to cut, in seconds on the timeline. This sentence is long enough to
        /// be wrapped over two lines.
        #[serde(default)]
        time_in_seconds: Option<f64>,
    }

    #[test]
    fn schemas_are_objects_describing_each_field() {
        let schema = input_schema_for::<ExampleParameters>();
        assert_eq!(schema["type"], "object");
        assert!(!schema.contains_key("$schema"));
        assert!(!schema.contains_key("description"));
        let clip_id = &schema["properties"]["clip_id"];
        assert_eq!(clip_id["description"], "The clip to cut, for example 4.");
        assert_eq!(schema["required"], serde_json::json!(["clip_id"]));
        let time = schema["properties"]["time_in_seconds"]["description"]
            .as_str()
            .unwrap();
        assert!(!time.contains('\n'), "{time}");
    }

    #[test]
    fn tools_without_parameters_have_an_empty_object_schema() {
        let schema = input_schema_for::<NoParameters>();
        assert_eq!(schema["type"], "object");
        assert!(read_arguments::<NoParameters>("undo", Value::Null).is_ok());
    }

    #[test]
    fn valid_arguments_are_read() {
        let parameters = read_arguments::<ExampleParameters>(
            "split_clip",
            serde_json::json!({ "clip_id": 4, "time_in_seconds": 2.5 }),
        )
        .unwrap();
        assert_eq!(parameters.clip_id, 4);
        assert_eq!(parameters.time_in_seconds, Some(2.5));
    }

    #[test]
    fn argument_mistakes_are_explained() {
        let error =
            read_arguments::<ExampleParameters>("split_clip", serde_json::json!({ "clipId": 4 }))
                .unwrap_err();
        assert!(error.to_string().contains("split_clip"), "{error}");
        assert!(error.to_string().contains("clipId"), "{error}");
    }
}
