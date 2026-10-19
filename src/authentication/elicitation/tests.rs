use serde_json::json;

use super::*;

fn form(schema: Value) -> Value {
    json!({"mode":"form","message":"Configure workspace","requestId":7,"requestedSchema":schema,"_meta":{"future":"retained"}})
}

#[test]
fn defaults_and_typed_values_cover_all_supported_controls() {
    let raw = form(json!({"type":"object","properties":{
        "name":{"type":"string","minLength":2,"maxLength":5,"pattern":"^[a-z]+$","default":"atlas"},
        "limit":{"type":"integer","minimum":1,"maximum":10,"default":3},
        "ratio":{"type":"number","minimum":0.1,"maximum":1.0,"default":0.5},
        "enabled":{"type":"boolean","default":true},
        "theme":{"type":"string","oneOf":[{"const":"dark","title":"Dark"},{"const":"light","title":"Light"}],"default":"dark"},
        "tools":{"type":"array","items":{"type":"string","enum":["rust","shell"]},"minItems":1,"maxItems":2,"default":["rust"]}
    },"required":["name","limit","tools"]}));
    let request = parse_request(&raw).expect("supported flat form");
    let Mode::Form(form) = &request.mode else {
        panic!("form expected")
    };
    assert_eq!(form.fields.len(), 6);
    assert_eq!(request.raw, raw);
    validate_response(
        &request,
        &json!({"action":"accept","content":form.defaults()}),
    )
    .expect("valid defaults");
    let invalid = json!({"name":"a","limit":0,"tools":["other"]});
    assert!(validate_response(&request, &json!({"action":"accept","content":invalid})).is_err());
    assert!(
        validate_response(
            &request,
            &json!({"action":"accept","content":{"name":"atlas","limit":2.5,"tools":["rust"]}})
        )
        .is_err()
    );
    assert!(
        validate_response(
            &request,
            &json!({"action":"accept","content":{"name":"atlas","limit":2,"tools":["rust","rust"]}})
        )
        .is_err()
    );
    assert!(validate_response(&request, &json!({"action":"accept","content":{"name":"atlas","limit":2,"tools":["rust"],"extra":true}})).is_err());
}

#[test]
fn required_fields_are_distinct_from_optional_empty_and_false_values() {
    let request = parse_request(&form(json!({"properties":{
        "text":{"type":"string"},"enabled":{"type":"boolean"},"optional":{"type":"string"}
    },"required":["text","enabled"]})))
    .expect("valid form");
    validate_response(
        &request,
        &json!({"action":"accept","content":{"text":"","enabled":false}}),
    )
    .expect("false and empty are supplied");
    assert!(
        validate_response(&request, &json!({"action":"accept","content":{"text":""}})).is_err()
    );
    validate_response(
        &request,
        &json!({"action":"decline","content":{"ignored":"anything"}}),
    )
    .expect("decline ignores content");
    validate_response(&request, &json!({"action":"cancel"})).expect("dismissal allowed");
}

#[test]
fn explicit_sensitive_markers_are_rejected_without_guessing_from_names() {
    for marker in [
        json!({"format":"password"}),
        json!({"writeOnly":true}),
        json!({"x-sensitive":true}),
        json!({"_meta":{"sensitive":true}}),
    ] {
        let mut property = marker;
        property["type"] = json!("string");
        assert!(parse_request(&form(json!({"properties":{"value":property}}))).is_err());
    }
    let named = form(
        json!({"properties":{"api_key_documentation":{"type":"string","description":"Name of the documentation section"}}}),
    );
    parse_request(&named).expect("names cannot determine credential intent");
}

#[test]
fn cancellation_ignores_form_values_and_accept_still_requires_conforming_content() {
    let request = parse_request(&form(
        json!({"properties":{"name":{"type":"string"}},"required":["name"]}),
    ))
    .expect("valid required form");
    validate_response(&request, &json!({"action":"cancel","content":"unused"}))
        .expect("cancel contents ignored");
    validate_response(
        &request,
        &json!({"action":"decline","content":{"name":123}}),
    )
    .expect("decline contents ignored");
    for action in ["timeout", "submit", ""] {
        assert!(validate_response(&request, &json!({"action":action})).is_err());
    }
    assert!(validate_response(&request, &json!({"action":"accept"})).is_err());
    assert!(validate_response(&request, &json!({"action":"accept","content":null})).is_err());
}
