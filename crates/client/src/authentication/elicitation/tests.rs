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

#[test]
fn length_and_selection_bounds_share_shape_and_order_validation() {
    for (property, min, max) in [
        (json!({"type":"string"}), "minLength", "maxLength"),
        (
            json!({"type":"array","items":{"type":"string","enum":["a","b"]}}),
            "minItems",
            "maxItems",
        ),
    ] {
        for (minimum, maximum, expected) in [
            (json!(null), json!(null), None),
            (json!(0), json!(null), None),
            (json!(null), json!(2), None),
            (json!(2), json!(2), None),
            (json!(3), json!(2), Some(format!("{min} 大于 {max}"))),
            (
                json!(-1),
                json!("invalid"),
                Some(format!("{min} 必须是非负整数")),
            ),
            (
                json!(null),
                json!(1.5),
                Some(format!("{max} 必须是非负整数")),
            ),
            (
                json!(null),
                json!("2"),
                Some(format!("{max} 必须是非负整数")),
            ),
        ] {
            let mut property = property.clone();
            property[min] = minimum;
            property[max] = maximum;
            let result = parse_request(&form(json!({"properties":{"value":property}})));
            match expected {
                Some(expected) => {
                    let Error::Elicitation(reason) = result.expect_err("invalid bounds") else {
                        panic!("expected schema error")
                    };
                    assert_eq!(reason, expected);
                }
                None => {
                    result.expect("valid bounds");
                }
            }
        }
    }
}

#[test]
fn defaults_and_submitted_fields_use_the_same_validation() {
    let request = parse_request(&form(json!({"properties":{
        "name":{"type":"string","title":"Name","minLength":2,"default":"a"},
        "tools":{"type":"array","title":"Tools","items":{"enum":["rust"]},"default":["rust","rust"]},
        "count":{"type":"integer","default":2}
    }})))
    .expect("invalid defaults do not invalidate the schema");
    let Mode::Form(form) = &request.mode else {
        panic!("form expected")
    };
    assert_eq!(
        form.defaults(),
        json!({"count":2}).as_object().expect("object").clone()
    );
    for (name, value, expected) in [
        ("name", json!("a"), "Name：长度或选择数量超出允许范围"),
        (
            "tools",
            json!(["rust", "rust"]),
            "Tools：请选择互不重复的已提供选项",
        ),
    ] {
        let field = form
            .fields
            .iter()
            .find(|field| field.name == name)
            .expect("field");
        let Error::Response(reason) = field.validate(&value).expect_err("invalid field") else {
            panic!("expected response error")
        };
        assert_eq!(reason, expected);
        let mut content = Map::new();
        content.insert(name.into(), value);
        let Error::Response(reason) = form.validate(&content).expect_err("invalid form") else {
            panic!("expected response error")
        };
        assert_eq!(reason, expected);
    }
}
