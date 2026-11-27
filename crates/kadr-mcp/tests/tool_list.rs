//! What Claude sees when it connects: the server's identity, its instructions and
//! a complete, well-described tool list.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::HashSet;

use support::TestServer;

/// The tools section 5 of the specification asks for.
const REQUIRED_TOOLS: &[&str] = &[
    "find_media_files",
    "import_media",
    "probe_media",
    "get_project_info",
    "list_media",
    "get_timeline",
    "get_clip",
    "get_frame",
    "get_contact_sheet",
    "detect_silences",
    "create_project",
    "open_project",
    "add_clip",
    "remove_clip",
    "move_clip",
    "trim_clip",
    "split_clip",
    "set_clip_speed",
    "set_clip_volume",
    "set_transform",
    "add_track",
    "remove_track",
    "add_text",
    "edit_text",
    "add_transition",
    "add_color_adjustment",
    "blur_region",
    "get_blur_status",
    "undo",
    "redo",
    "save_project",
    "set_project_settings",
    "export",
    "get_export_status",
];

#[tokio::test(flavor = "multi_thread")]
async fn the_server_introduces_itself_with_instructions() {
    let server = TestServer::start().await;
    let info = server
        .client
        .peer_info()
        .expect("the server answered initialize");
    assert_eq!(
        info.server_info.as_ref().map(|server| server.name.as_str()),
        Some("kadr")
    );
    let instructions = info.instructions.clone().unwrap_or_default();
    assert!(instructions.contains("NOT available"), "{instructions}");
    assert!(instructions.contains("top-left"), "{instructions}");
}

#[tokio::test(flavor = "multi_thread")]
async fn every_required_tool_is_listed_with_a_schema_and_a_description() {
    let server = TestServer::start().await;
    let tools = server.client.list_all_tools().await.unwrap();
    let names: HashSet<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names.len(), tools.len(), "tool names must be unique");
    for required in REQUIRED_TOOLS {
        assert!(names.contains(required), "{required} is missing");
    }

    for tool in &tools {
        let description = tool.description.as_deref().unwrap_or_default();
        assert!(
            description.len() > 40,
            "{} has a too short description",
            tool.name
        );
        assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
        assert!(
            tool.input_schema.contains_key("properties"),
            "{}",
            tool.name
        );
        assert!(
            tool.annotations.is_some(),
            "{} has no annotations",
            tool.name
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn descriptions_state_units_and_coordinates() {
    let server = TestServer::start().await;
    let tools = server.client.list_all_tools().await.unwrap();
    let description_of = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .and_then(|tool| tool.description.clone())
            .unwrap_or_default()
    };
    assert!(description_of("split_clip").contains("seconds"));
    assert!(description_of("get_frame").contains("top-left"));
    assert!(description_of("get_frame").contains("1024"));
    assert!(description_of("set_transform").contains("pixels"));
    assert!(description_of("find_media_files").contains("NOT available"));
    assert!(description_of("export").contains("overwrite"));

    // Every parameter is described, with its unit when it is a time.
    for tool in &tools {
        let properties = tool.input_schema["properties"].as_object().unwrap();
        for (parameter_name, parameter) in properties {
            let parameter_description = parameter["description"].as_str().unwrap_or_default();
            assert!(
                !parameter_description.is_empty(),
                "{}.{parameter_name} has no description",
                tool.name
            );
            if parameter_name.ends_with("_in_seconds") {
                assert!(
                    parameter_description.contains("second"),
                    "{}.{parameter_name}: {parameter_description}",
                    tool.name
                );
            }
        }
    }
}
