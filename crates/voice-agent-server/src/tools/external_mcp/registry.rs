//! External MCP tool vocabulary: namespace normalization, supported-schema validation and the
//! immutable per-server catalog one admission snapshot publishes.
//!
//! Device MCP and External MCP never share a name: Device tools answer to `self.<tool>` and
//! External tools to `external.<server_key>.<tool>`.  A normalized name is therefore a display
//! name only — [`ToolOrigin`] carries the original wire name a call is actually made with, so a
//! model can never choose where a tool runs.

use std::collections::HashSet;

use serde_json::Value;

/// One namespace segment is what an LLM-visible name may spend on a server key or a tool name.
/// Longer input is rejected outright: truncating two distinct names onto one prefix would create
/// exactly the collision this module exists to prevent.
pub const MAX_EXTERNAL_TOOL_SEGMENT: usize = 64;

/// Every normalization step is a pure, bounded mapping of one wire name onto one segment.
pub fn normalize_external_tool_segment(value: &str) -> Option<String> {
    let mut normalized = String::new();
    // Every disallowed run, and every already-underscore run, collapses into one pending break.
    // A break only becomes a separator once something has been written, which is what trims a
    // leading one; never setting it after the last kept character trims a trailing one.
    let mut separator_pending = false;
    for character in value.chars().flat_map(|character| character.to_lowercase()) {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            if separator_pending && !normalized.is_empty() {
                normalized.push('_');
            }
            separator_pending = false;
            normalized.push(character);
        } else {
            separator_pending = true;
        }
    }
    let first = normalized.chars().next()?;
    let segment = if first.is_ascii_lowercase() {
        normalized
    } else {
        format!("x_{normalized}")
    };
    (segment.len() <= MAX_EXTERNAL_TOOL_SEGMENT).then_some(segment)
}

/// Bounds of the accepted JSON Schema subset.  They are fixed rather than configured: the
/// subset is a language the LLM conversion understands, not an operator policy.
const MAX_SCHEMA_DEPTH: usize = 12;
const MAX_SCHEMA_NODES: usize = 512;
const MAX_SCHEMA_PROPERTIES_TOTAL: usize = 256;
const MAX_SCHEMA_PROPERTIES_PER_OBJECT: usize = 64;
const MAX_SCHEMA_REQUIRED: usize = 64;
const MAX_SCHEMA_ENUM_ITEMS: usize = 128;

/// The only keywords the conversion can honor.  Everything else — `$ref`, `$defs`, composition,
/// recursion, dynamic and unevaluated keywords — has no defined meaning here, so an untrusted
/// schema that uses one is refused rather than silently reshaped.
const ALLOWED_SCHEMA_KEYWORDS: &[&str] = &[
    "type",
    "description",
    "properties",
    "required",
    "additionalProperties",
    "enum",
    "const",
    "items",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
];

const SUPPORTED_SCHEMA_TYPES: &[&str] =
    &["string", "number", "integer", "boolean", "object", "array"];

/// Counters shared by every node of one schema, so the caps bound the whole document rather than
/// each level independently.
#[derive(Default)]
struct SchemaBudget {
    nodes: usize,
    properties: usize,
}

/// Why one tool schema cannot be converted.  Bounded and content-free: nothing derived from the
/// remote document ever reaches this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SchemaRejection {
    #[error("schema_not_object")]
    NotObject,
    #[error("schema_keyword_unsupported")]
    UnsupportedKeyword,
    #[error("schema_type_unsupported")]
    UnsupportedType,
    #[error("schema_depth_exceeded")]
    DepthExceeded,
    #[error("schema_node_cap_exceeded")]
    NodeCapExceeded,
    #[error("schema_property_cap_exceeded")]
    PropertyCapExceeded,
    #[error("schema_required_cap_exceeded")]
    RequiredCapExceeded,
    #[error("schema_enum_cap_exceeded")]
    EnumCapExceeded,
    #[error("schema_required_unknown_property")]
    RequiredUnknownProperty,
}

/// Accepts only the bounded supported subset.  The check is total and never rewrites: a schema it
/// rejects costs the whole server its tools, because a half-understood schema would silently
/// change the arguments a tool is called with.
pub fn validate_external_tool_schema(schema: &Value) -> Result<(), SchemaRejection> {
    let mut budget = SchemaBudget::default();
    validate_schema_node(schema, 1, &mut budget, true)
}

fn validate_schema_node(
    schema: &Value,
    depth: usize,
    budget: &mut SchemaBudget,
    root: bool,
) -> Result<(), SchemaRejection> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(SchemaRejection::DepthExceeded);
    }
    budget.nodes += 1;
    if budget.nodes > MAX_SCHEMA_NODES {
        return Err(SchemaRejection::NodeCapExceeded);
    }
    let object = schema.as_object().ok_or(SchemaRejection::NotObject)?;
    if root && object.get("type").and_then(Value::as_str) != Some("object") {
        return Err(SchemaRejection::UnsupportedType);
    }
    if object
        .keys()
        .any(|key| !ALLOWED_SCHEMA_KEYWORDS.contains(&key.as_str()))
    {
        return Err(SchemaRejection::UnsupportedKeyword);
    }
    let declared = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or(SchemaRejection::UnsupportedType)?;
    if !SUPPORTED_SCHEMA_TYPES.contains(&declared) {
        return Err(SchemaRejection::UnsupportedType);
    }
    if let Some(required) = object.get("required") {
        let entries = required
            .as_array()
            .ok_or(SchemaRejection::UnsupportedKeyword)?;
        if entries.len() > MAX_SCHEMA_REQUIRED {
            return Err(SchemaRejection::RequiredCapExceeded);
        }
    }
    if let Some(values) = object.get("enum")
        && values
            .as_array()
            .ok_or(SchemaRejection::UnsupportedKeyword)?
            .len()
            > MAX_SCHEMA_ENUM_ITEMS
    {
        return Err(SchemaRejection::EnumCapExceeded);
    }
    for numeric in ["minimum", "maximum", "minLength", "maxLength"] {
        if let Some(bound) = object.get(numeric)
            && !bound.is_number()
            && !bound.is_null()
        {
            return Err(SchemaRejection::UnsupportedKeyword);
        }
    }
    if let Some(extra) = object.get("additionalProperties") {
        // Only the closed form is representable; an open form would promise arguments the
        // conversion cannot describe.
        if extra != &Value::Bool(false) {
            return Err(SchemaRejection::UnsupportedKeyword);
        }
    }

    let properties = match object.get("properties") {
        Some(value) => Some(
            value
                .as_object()
                .ok_or(SchemaRejection::UnsupportedKeyword)?,
        ),
        None => None,
    };
    if let Some(required) = object.get("required") {
        for entry in required.as_array().expect("checked above") {
            let name = entry.as_str().ok_or(SchemaRejection::UnsupportedKeyword)?;
            // A `required` entry with no property to satisfy names something the schema does not
            // describe, so the tool would be advertised as taking an argument it cannot take.
            if !properties.is_some_and(|properties| properties.contains_key(name)) {
                return Err(SchemaRejection::RequiredUnknownProperty);
            }
        }
    }
    let Some(properties) = properties else {
        return match object.get("items") {
            Some(items) => validate_schema_node(items, depth + 1, budget, false),
            None => Ok(()),
        };
    };
    if properties.len() > MAX_SCHEMA_PROPERTIES_PER_OBJECT {
        return Err(SchemaRejection::PropertyCapExceeded);
    }
    budget.properties += properties.len();
    if budget.properties > MAX_SCHEMA_PROPERTIES_TOTAL {
        return Err(SchemaRejection::PropertyCapExceeded);
    }
    for value in properties.values() {
        validate_schema_node(value, depth + 1, budget, false)?;
    }
    match object.get("items") {
        Some(items) => validate_schema_node(items, depth + 1, budget, false),
        None => Ok(()),
    }
}

/// Where a tool call really goes.  Routing resolves through this, never through the LLM-visible
/// name, so a sanitized name can only ever be a label.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ToolOrigin {
    Device {
        original_name: String,
    },
    ExternalMcp {
        server_key: String,
        original_name: String,
    },
}

/// One tool exactly as a session may use it for the rest of its life.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedExternalTool {
    pub llm_name: String,
    pub original_name: String,
    pub description: String,
    pub input_schema: Value,
}

/// The validated, collision-free tool set of one MCP server.
///
/// It is built once, at admission, and never grows, shrinks or is pruned: a `tools/call` that
/// times out or is refused is telemetry of one invocation, not evidence about the catalog.
/// The published tools of one server, plus what publication had to leave out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExternalToolCatalog {
    tools: Vec<ResolvedExternalTool>,
    dropped: usize,
}

impl ExternalToolCatalog {
    /// Publishes one server's tools under a fixed namespace.
    ///
    /// A name that has no representable segment is dropped, because the model could not have
    /// called it anyway.  Two names that normalize alike leave no way to tell the tools apart, so
    /// nothing is published rather than picking a first or last winner: the caller decides what to
    /// do with a server whose catalog did not survive.
    pub fn publish(
        namespace: &str,
        discovered: Vec<(String, String, Value)>,
    ) -> Result<Self, ToolPublishError> {
        let mut tools = Vec::with_capacity(discovered.len());
        let mut seen = HashSet::with_capacity(discovered.len());
        let mut dropped = 0;
        for (original_name, description, input_schema) in discovered {
            let Some(segment) = normalize_external_tool_segment(&original_name) else {
                dropped += 1;
                continue;
            };
            let llm_name = format!("{namespace}.{segment}");
            if !seen.insert(llm_name.clone()) {
                return Err(ToolPublishError::NameCollision);
            }
            tools.push(ResolvedExternalTool {
                llm_name,
                original_name,
                description,
                input_schema,
            });
        }
        Ok(Self { tools, dropped })
    }

    pub fn tools(&self) -> &[ResolvedExternalTool] {
        &self.tools
    }

    /// How many tools the server announced that no LLM-visible name could have stood for.  The
    /// count is the diagnostic; the names themselves are the remote server's content and never
    /// travel.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn find(&self, llm_name: &str) -> Option<&ResolvedExternalTool> {
        self.tools.iter().find(|tool| tool.llm_name == llm_name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ToolPublishError {
    #[error("mcp_tool_name_collision")]
    NameCollision,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_pure_and_bounded_with_fixed_vectors() {
        assert_eq!(
            normalize_external_tool_segment("ABC").as_deref(),
            Some("abc")
        );
        assert_eq!(
            normalize_external_tool_segment("foo-bar").as_deref(),
            Some("foo_bar")
        );
        assert_eq!(
            normalize_external_tool_segment("123tool").as_deref(),
            Some("x_123tool")
        );
        assert_eq!(
            normalize_external_tool_segment("__tool").as_deref(),
            Some("tool")
        );
        assert_eq!(
            normalize_external_tool_segment("Home-Assistant").as_deref(),
            Some("home_assistant")
        );
        assert_eq!(
            normalize_external_tool_segment("Light/Turn-On").as_deref(),
            Some("light_turn_on")
        );
        assert_eq!(
            normalize_external_tool_segment("a__b").as_deref(),
            Some("a_b")
        );
        // Unicode-only, empty and over-long input have no representable segment.
        assert_eq!(normalize_external_tool_segment("あいう"), None);
        assert_eq!(normalize_external_tool_segment(""), None);
        assert_eq!(normalize_external_tool_segment("---"), None);
        assert_eq!(
            normalize_external_tool_segment(&"a".repeat(MAX_EXTERNAL_TOOL_SEGMENT + 1)),
            None
        );
        assert_eq!(
            normalize_external_tool_segment(&"a".repeat(MAX_EXTERNAL_TOOL_SEGMENT)).as_deref(),
            Some("a".repeat(MAX_EXTERNAL_TOOL_SEGMENT).as_str())
        );
    }

    #[test]
    fn an_unrepresentable_name_is_dropped_while_a_collision_loses_the_server() {
        let published = ExternalToolCatalog::publish(
            "external.home",
            vec![
                ("Light".into(), String::new(), object_schema()),
                ("---".into(), String::new(), object_schema()),
            ],
        )
        .expect("a name with no representable segment is simply dropped");
        assert_eq!(published.tools().len(), 1);
        assert_eq!(published.tools()[0].llm_name, "external.home.light");
        assert_eq!(published.tools()[0].original_name, "Light");

        assert_eq!(
            ExternalToolCatalog::publish(
                "external.home",
                vec![
                    ("foo-bar".into(), String::new(), object_schema()),
                    ("foo_bar".into(), String::new(), object_schema()),
                ],
            )
            .unwrap_err(),
            ToolPublishError::NameCollision
        );
        assert!(
            ExternalToolCatalog::publish(
                "external.home",
                vec![(
                    "x".repeat(MAX_EXTERNAL_TOOL_SEGMENT + 1),
                    String::new(),
                    object_schema()
                )],
            )
            .expect("an over-long name is dropped")
            .is_empty()
        );
    }

    fn object_schema() -> Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    fn schema_of(value: Value) -> Result<(), SchemaRejection> {
        validate_external_tool_schema(&value)
    }

    #[test]
    fn a_supported_subset_is_accepted_including_a_closed_object() {
        assert_eq!(schema_of(object_schema()), Ok(()));
        assert_eq!(
            schema_of(serde_json::json!({
                "type": "object",
                "description": "bounded",
                "properties": {
                    "city": {"type": "string", "minLength": 1, "maxLength": 64},
                    "days": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 7}},
                    "unit": {"type": "string", "enum": ["c", "f"], "const": "c"}
                },
                "required": ["city"],
                "additionalProperties": false
            })),
            Ok(())
        );
    }

    #[test]
    fn every_shape_outside_the_supported_subset_is_refused() {
        let unsupported = [
            serde_json::json!({"type": "object", "$ref": "#/definitions/tool"}),
            serde_json::json!({"type": "object", "$defs": {}}),
            serde_json::json!({"type": "object", "definitions": {}}),
            serde_json::json!({"type": "object", "allOf": [{"type": "object"}]}),
            serde_json::json!({"type": "object", "anyOf": [{"type": "object"}]}),
            serde_json::json!({"type": "object", "oneOf": [{"type": "object"}]}),
            serde_json::json!({"type": "object", "not": {"type": "string"}}),
            serde_json::json!({"type": "object", "if": {"type": "object"}}),
            serde_json::json!({"type": "object", "then": {"type": "object"}}),
            serde_json::json!({"type": "object", "else": {"type": "object"}}),
            serde_json::json!({"type": "object", "unevaluatedProperties": false}),
            serde_json::json!({"type": "object", "dynamicRef": "#node"}),
            serde_json::json!({"type": "object", "recursiveRef": "#"}),
            serde_json::json!({"type": "array"}),
            serde_json::json!({"type": "object", "properties": "not-an-object"}),
            serde_json::json!({"type": "object", "required": ["absent"]}),
            serde_json::json!({"type": "object", "additionalProperties": true}),
        ];
        for schema in unsupported {
            assert!(
                schema_of(schema.clone()).is_err(),
                "{schema} must be refused rather than reshaped"
            );
        }
        assert_eq!(
            schema_of(serde_json::json!({})),
            Err(SchemaRejection::UnsupportedType)
        );
        assert_eq!(
            schema_of(Value::String("no".into())),
            Err(SchemaRejection::NotObject)
        );
    }

    #[test]
    fn structural_caps_bound_a_document_instead_of_each_level_separately() {
        let deep = {
            let mut schema = object_schema();
            for _ in 0..MAX_SCHEMA_DEPTH {
                schema = serde_json::json!({"type": "object", "items": schema});
            }
            schema
        };
        assert_eq!(schema_of(deep), Err(SchemaRejection::DepthExceeded));

        let wide = serde_json::json!({
            "type": "object",
            "properties": (0..MAX_SCHEMA_PROPERTIES_PER_OBJECT + 1)
                .map(|index| (format!("p{index}"), serde_json::json!({"type": "string"})))
                .collect::<serde_json::Map<String, Value>>()
        });
        assert_eq!(schema_of(wide), Err(SchemaRejection::PropertyCapExceeded));

        let required = serde_json::json!({
            "type": "object",
            "required": (0..MAX_SCHEMA_REQUIRED + 1).map(|index| format!("p{index}")).collect::<Vec<_>>()
        });
        assert_eq!(
            schema_of(required),
            Err(SchemaRejection::RequiredCapExceeded)
        );

        let enumerated = serde_json::json!({
            "type": "object",
            "properties": {
                "unit": {"type": "string", "enum": (0..MAX_SCHEMA_ENUM_ITEMS + 1).map(Value::from).collect::<Vec<_>>()}
            }
        });
        assert_eq!(schema_of(enumerated), Err(SchemaRejection::EnumCapExceeded));
    }
}
