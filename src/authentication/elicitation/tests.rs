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
fn titled_multiselect_uses_any_of_options_and_checks_selection_count() {
    let raw = form(
        json!({"properties":{"tools":{"type":"array","items":{"anyOf":[
        {"const":"rust","title":"Rust","description":"Build code"},
        {"const":"shell","title":"Shell"}
    ]},"minItems":1,"maxItems":1}},"required":["tools"]}),
    );
    let request = parse_request(&raw).expect("titled multi-select supported");
    let Mode::Form(form) = &request.mode else {
        panic!("form expected")
    };
    assert!(
        matches!(&form.fields[0].kind, Kind::MultiSelect {options} if options[0].title == "Rust")
    );
    validate_response(
        &request,
        &json!({"action":"accept","content":{"tools":["rust"]}}),
    )
    .expect("one choice valid");
    assert!(
        validate_response(
            &request,
            &json!({"action":"accept","content":{"tools":["rust","shell"]}})
        )
        .is_err()
    );
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
fn malformed_future_or_nested_schema_is_not_rendered_as_text() {
    for schema in [
        json!({"type":"array"}),
        json!({"properties":{"nested":{"type":"object","properties":{}}}}),
        json!({"properties":{"future":{"type":"location"}}}),
        json!({"properties":{"n":{"type":"number","minimum":"0"}}}),
        json!({"properties":{"n":{"type":"integer","minimum":2,"maximum":1}}}),
        json!({"properties":{"s":{"type":"string","minLength":-1}}}),
        json!({"properties":{"s":{"type":"string","pattern":"(?=unsupported)"}}}),
        json!({"properties":{"s":{"type":"string","enum":["a","a"]}}}),
        json!({"properties":{"s":{"type":"boolean","enum":[true]}}}),
        json!({"anyOf":[{"properties":{}}]}),
        json!({"properties":{"s":{"type":"string"}},"required":["unknown"]}),
    ] {
        assert!(
            parse_request(&form(schema.clone())).is_err(),
            "unexpected schema accepted: {schema}"
        );
    }
    let future = json!({"mode":"future","message":"Do something","requestId":7,"extra":{}});
    assert!(parse_request(&future).is_err());
    let omitted = json!({"message":"Configure","requestId":7,"requestedSchema":{}});
    assert!(parse_request(&omitted).is_err());
}

#[test]
fn invalid_defaults_are_ignored_and_original_values_retained() {
    let request = parse_request(&form(json!({"properties":{
        "count":{"type":"integer","minimum":2,"default":1},
        "state":{"type":"boolean","default":"true"},
        "choice":{"type":"string","enum":["a"],"default":"other"}
    }})))
    .expect("invalid optional defaults do not invalidate whole form");
    let Mode::Form(form) = request.mode else {
        panic!("form expected")
    };
    assert!(form.defaults().is_empty());
    let choice = form
        .fields
        .iter()
        .find(|field| field.name == "choice")
        .expect("declared choice field retained");
    assert_eq!(choice.raw["default"], "other");
}

#[test]
fn full_url_and_host_are_preserved_without_navigating() {
    let raw = json!({"mode":"url","message":"Authorize","requestId":"auth-1","elicitationId":"opaque-1","url":"https://accounts.example.com/authorize?state=123"});
    let request = parse_request(&raw).expect("valid external authorization");
    let Mode::Url(url) = &request.mode else {
        panic!("url expected")
    };
    assert_eq!(url.host, "accounts.example.com");
    assert_eq!(url.url, raw["url"]);
    assert_eq!(url.elicitation_id, "opaque-1");
    validate_response(&request, &json!({"action":"accept"})).expect("consent only");
    assert!(
        validate_response(
            &request,
            &json!({"action":"accept","content":{"token":"must not be forwarded"}})
        )
        .is_err()
    );
    for url in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "https://user:password@example.com/",
        " https://example.com/",
    ] {
        let mut raw = raw.clone();
        raw["url"] = json!(url);
        assert!(parse_request(&raw).is_err(), "invalid URL accepted: {url}");
    }
}

#[test]
fn scope_is_one_connection_request_or_session_with_optional_tool() {
    let mut raw = form(json!({}));
    raw["sessionId"] = json!("session-1");
    assert!(parse_request(&raw).is_err());
    raw.as_object_mut()
        .expect("request object")
        .remove("requestId");
    raw["toolCallId"] = json!("tool-1");
    let request = parse_request(&raw).expect("valid session scope");
    assert_eq!(
        request.scope,
        json!({"sessionId":"session-1","toolCallId":"tool-1"})
    );
}

#[test]
fn string_formats_validate_dates_uris_and_profile_email() {
    let request = parse_request(&form(json!({"properties":{
        "date":{"type":"string","format":"date"},
        "datetime":{"type":"string","format":"date-time"},
        "email":{"type":"string","format":"email"},
        "uri":{"type":"string","format":"uri"}
    }})))
    .expect("supported formats");
    validate_response(&request,&json!({"action":"accept","content":{
        "date":"2024-02-29","datetime":"2026-10-09T14:30:00+08:00","email":"atlas@example.com","uri":"mailto:atlas@example.com"
    }})).expect("valid formats");
    for content in [
        json!({"date":"2025-02-29"}),
        json!({"datetime":"2026-10-09T25:00:00Z"}),
        json!({"email":"missing-at"}),
        json!({"email":"Atlas <atlas@example.com>"}),
        json!({"email":"a@b@example.com"}),
        json!({"date":"+2026-10-09"}),
        json!({"uri":"no scheme"}),
    ] {
        assert!(
            validate_response(&request, &json!({"action":"accept","content":content})).is_err()
        );
    }
}

#[test]
fn opaque_scope_ids_are_retained_and_invalid_scopes_are_rejected() {
    let mut raw = form(json!({"properties":{}}));
    raw["requestId"] = json!("request:/opaque?42");
    let request = parse_request(&raw).expect("opaque request ID supported");
    assert_eq!(request.scope, json!({"requestId":"request:/opaque?42"}));
    for scope in [
        json!({"requestId":1.5}),
        json!({"requestId":false}),
        json!({"requestId":1,"sessionId":"session"}),
        json!({"requestId":1,"toolCallId":"tool"}),
        json!({"sessionId":"session","toolCallId":null}),
        json!({}),
    ] {
        let mut raw = raw.clone();
        let object = raw.as_object_mut().expect("request object");
        object.remove("requestId");
        object.extend(scope.as_object().expect("scope object").clone());
        assert!(
            parse_request(&raw).is_err(),
            "invalid scope accepted: {scope}"
        );
    }
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
fn integer_constraints_preserve_precision_above_float_integer_range() {
    let request = parse_request(&form(json!({"properties":{"number":{"type":"integer","minimum":9007199254740993_i64,"maximum":9007199254740994_i64}}})))
        .expect("exact integer range");
    validate_response(
        &request,
        &json!({"action":"accept","content":{"number":9007199254740993_i64}}),
    )
    .expect("minimum included");
    assert!(
        validate_response(
            &request,
            &json!({"action":"accept","content":{"number":9007199254740992_i64}})
        )
        .is_err()
    );
    assert!(parse_request(&form(json!({"properties":{"number":{"type":"integer","minimum":9007199254740993_i64,"maximum":9007199254740992_i64}}}))).is_err());
}
