use basr::authentication::elicitation::{Mode, parse_request};
use serde_json::json;

use super::parse_input;

#[test]
fn terminal_choices_return_agent_values_and_validate_constraints() {
    let request = parse_request(&json!({
        "mode":"form","requestId":"configure","message":"Project settings",
        "requestedSchema":{"type":"object","properties":{
            "model":{"type":"string","enum":["small","large"]},
            "count":{"type":"integer","minimum":1},
            "enabled":{"type":"boolean"}
        }}
    }))
    .expect("supported schema");
    let Mode::Form(form) = request.mode else {
        panic!("form");
    };
    let field = |name: &str| {
        form.fields
            .iter()
            .find(|field| field.name == name)
            .expect("field")
    };
    assert_eq!(
        parse_input(field("model"), "2").expect("choice"),
        Some(json!("large"))
    );
    assert!(parse_input(field("model"), "3").is_err());
    let zero = parse_input(field("count"), "0")
        .expect("number")
        .expect("value");
    assert!(field("count").validate(&zero).is_err());
    let fraction = parse_input(field("count"), "1.5")
        .expect("number")
        .expect("value");
    assert!(field("count").validate(&fraction).is_err());
    assert_eq!(
        parse_input(field("enabled"), "false").expect("boolean"),
        Some(json!(false))
    );
    assert_eq!(
        parse_input(field("model"), "").expect("optional omission"),
        None
    );
}
