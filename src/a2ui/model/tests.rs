use super::*;

fn store_with(components: Value) -> Store {
    let mut s = Store::default();
    s.apply("a2ui:createSurface", &json!({"surfaceId": "s1"}));
    s.apply(
        "a2ui:updateComponents",
        &json!({"surfaceId": "s1", "components": components}),
    );
    s
}

fn call(name: &str, args: Value) -> Value {
    json!({"call": name, "args": args})
}

fn eval(model: &Value, v: Value) -> Option<Value> {
    execute_call(&v, &Ctx::new(model, None))
}

// ── almacén ──

#[test]
fn root_is_the_first_unreferenced_component() {
    let s = store_with(json!([
        {"id": "a", "component": "Text", "text": "x"},
        {"id": "root", "component": "Column", "children": ["a"]},
    ]));
    assert_eq!(s.get("s1").unwrap().root.as_deref(), Some("root"));
}

#[test]
fn root_is_kept_while_it_exists() {
    let mut s = store_with(json!([{"id": "root", "component": "Text", "text": "x"}]));
    s.apply(
        "a2ui:updateComponents",
        &json!({"surfaceId": "s1", "components": [{"id": "other", "component": "Text"}]}),
    );
    assert_eq!(s.get("s1").unwrap().root.as_deref(), Some("root"));
}

#[test]
fn nested_component_form_is_flattened() {
    let s = store_with(json!([{"id": "c", "component": {"Card": {"child": "x"}}}]));
    let def = s.get("s1").unwrap().component("c").unwrap();
    assert_eq!(def["component"], "Card");
    assert_eq!(def["child"], "x");
}

#[test]
fn same_id_replaces_the_whole_definition() {
    let mut s = store_with(json!([{"id": "t", "component": "Text", "text": "a", "variant": "h1"}]));
    s.apply(
        "a2ui:updateComponents",
        &json!({"surfaceId": "s1", "components": [{"id": "t", "component": "Text", "text": "b"}]}),
    );
    let def = s.get("s1").unwrap().component("t").unwrap();
    assert_eq!(def["text"], "b");
    assert!(def.get("variant").is_none(), "no se mezclan propiedades");
    assert_eq!(s.get("s1").unwrap().order, vec!["t"]);
}

#[test]
fn create_surface_replaces_an_existing_one() {
    let mut s = store_with(json!([{"id": "t", "component": "Text"}]));
    s.apply("a2ui:createSurface", &json!({"surfaceId": "s1"}));
    assert!(s.get("s1").unwrap().components.is_empty());
    assert_eq!(s.surfaces.len(), 1);
}

#[test]
fn unknown_surface_and_empty_components_are_ignored() {
    let mut s = Store::default();
    assert!(!s.apply(
        "a2ui:updateComponents",
        &json!({"surfaceId": "nope", "components": [{"id": "a"}]})
    ));
    s.apply("a2ui:createSurface", &json!({"surfaceId": "s1"}));
    assert!(!s.apply(
        "a2ui:updateComponents",
        &json!({"surfaceId": "s1", "components": []})
    ));
    assert!(!s.apply("a2ui:deleteSurface", &json!({"surfaceId": "nope"})));
    assert!(s.apply("a2ui:deleteSurface", &json!({"surfaceId": "s1"})));
    assert!(s.surfaces.is_empty());
}

// ── modelo de datos ──

#[test]
fn data_model_paths_create_replace_and_delete() {
    let mut s = store_with(json!([]));
    let up = |s: &mut Store, d: Value| s.apply("a2ui:updateDataModel", &d);
    up(
        &mut s,
        json!({"surfaceId": "s1", "path": "/user/name", "value": "Ana"}),
    );
    assert_eq!(s.get("s1").unwrap().data, json!({"user": {"name": "Ana"}}));
    up(&mut s, json!({"surfaceId": "s1", "path": "/user/name"}));
    assert_eq!(
        s.get("s1").unwrap().data,
        json!({"user": {}}),
        "sin value se borra la clave"
    );
    up(
        &mut s,
        json!({"surfaceId": "s1", "path": "/user/age", "value": null}),
    );
    assert_eq!(
        s.get("s1").unwrap().data["user"]["age"],
        Value::Null,
        "null es un valor"
    );
    up(&mut s, json!({"surfaceId": "s1", "value": {"a": 1}}));
    assert_eq!(
        s.get("s1").unwrap().data,
        json!({"a": 1}),
        "sin ruta reemplaza todo"
    );
}

#[test]
fn set_path_overwrites_primitives_on_the_way_and_indexes_arrays() {
    let mut m = json!({"a": 5, "list": [{"x": 1}, {"x": 2}]});
    set_path(&mut m, "/a/b", Some(json!(true)));
    assert_eq!(m["a"], json!({"b": true}));
    set_path(&mut m, "/list/1/x", Some(json!(9)));
    assert_eq!(m["list"][1]["x"], 9);
}

// ── bindings ──

#[test]
fn relative_paths_use_scope_then_fall_back_to_the_model() {
    let model = json!({"title": "global", "items": []});
    let scope = json!({"name": "item"});
    let ctx = Ctx::new(&model, Some(&scope));
    assert_eq!(dyn_string(Some(&json!({"path": "name"})), &ctx), "item");
    assert_eq!(dyn_string(Some(&json!({"path": "title"})), &ctx), "global");
    assert_eq!(dyn_string(Some(&json!({"path": "/title"})), &ctx), "global");
    assert_eq!(dyn_string(Some(&json!({"path": "missing"})), &ctx), "");
}

#[test]
fn dynamic_values_have_the_documented_defaults() {
    let model = json!({"n": 3, "s": "true", "list": ["a", 2], "one": "x"});
    let ctx = Ctx::new(&model, None);
    assert_eq!(
        dyn_number(Some(&json!({"path": "/s"})), &ctx),
        0.0,
        "una ruta no convierte texto"
    );
    assert_eq!(dyn_number(Some(&json!({"path": "/n"})), &ctx), 3.0);
    assert!(
        dyn_bool(Some(&json!({"path": "/s"})), &ctx),
        "\"true\" cuenta como verdadero"
    );
    assert_eq!(
        dyn_string_list(Some(&json!({"path": "/list"})), &ctx),
        vec!["a", "2"]
    );
    assert_eq!(
        dyn_string_list(Some(&json!({"path": "/one"})), &ctx),
        vec!["x"]
    );
    assert_eq!(dyn_string(None, &ctx), "");
}

#[test]
fn template_children_scope_objects_and_primitives() {
    let model = json!({"items": [{"title": "A"}, {"title": "B"}], "nums": [10, 20]});
    let ctx = Ctx::new(&model, None);
    let kids = children_of(&json!({"path": "/items", "componentId": "row"}), &ctx);
    assert_eq!(kids.len(), 2);
    assert_eq!(kids[1].key, "row_1");
    assert_eq!(kids[1].scope, Some(json!({"title": "B"})));
    let kids = children_of(
        &json!({"template": {"dataBinding": "/nums", "componentId": "n"}}),
        &ctx,
    );
    assert_eq!(kids[0].scope, Some(json!({"value": 10, "index": 0})));
    assert!(children_of(&json!({"path": "/nope", "componentId": "row"}), &ctx).is_empty());
}

#[test]
fn children_accept_every_documented_form() {
    let model = json!({});
    let ctx = Ctx::new(&model, None);
    let ids = |v: Value| {
        children_of(&v, &ctx)
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(json!("a")), vec!["a"]);
    assert_eq!(ids(json!(["a", "b"])), vec!["a", "b"]);
    assert_eq!(ids(json!({"explicitList": ["a"]})), vec!["a"]);
    assert_eq!(ids(json!({"array": ["b"]})), vec!["b"]);
}

// ── funciones ──

#[test]
fn format_string_interpolates_paths_and_keeps_unresolved() {
    let model = json!({"user": {"name": "Ana"}, "n": 5});
    let ctx = Ctx::new(&model, None);
    assert_eq!(
        format_string("Hola ${/user/name} (${/n})", &ctx),
        "Hola Ana (5)"
    );
    assert_eq!(format_string("x ${/missing} y", &ctx), "x ${/missing} y");
    assert_eq!(format_string("abierto ${/n", &ctx), "abierto ${/n");
    assert_eq!(
        format_string("${formatNumber(value:1234.5, precision:1)}", &ctx),
        "1,234.5"
    );
}

#[test]
fn format_string_call_resolves_the_template_argument() {
    let model = json!({"user": {"name": "Ana"}});
    let out = eval(
        &model,
        call("formatString", json!({"value": "Hola ${/user/name}"})),
    );
    assert_eq!(out, Some(json!("Hola Ana")));
}

#[test]
fn numbers_and_currency_follow_en_us() {
    assert_eq!(format_number(1234567.891, true, 2), "1,234,567.89");
    assert_eq!(format_number(1234.0, false, 0), "1234");
    assert_eq!(format_number(-1234.5, true, 1), "-1,234.5");
    assert_eq!(format_number(f64::NAN, true, 2), "0");
    assert_eq!(format_currency(1234.5, "USD"), "$1,234.50");
    assert_eq!(format_currency(-5.0, "EUR"), "-€5.00");
    assert_eq!(format_currency(7.0, "no"), "$7.00");
}

#[test]
fn dates_use_a_real_tokenizer() {
    assert_eq!(format_date("2026-03-07", "yyyy-MM-dd"), "2026-03-07");
    assert_eq!(format_date("2026-03-07", "d/M/yy"), "7/3/26");
    assert_eq!(format_date("2026-03-07T15:04:09", "HH:mm:ss"), "15:04:09");
    assert_eq!(format_date("2026-03-07T15:04:09", "hh:mm a"), "03:04 PM");
    // El nombre del mes no se vuelve a interpretar como tokens (error conocido de la UI web).
    assert_eq!(format_date("2026-03-07", "MMMM d, yyyy"), "March 7, 2026");
    assert_eq!(format_date("2026-03-07", "EEEE"), "Saturday");
    assert_eq!(format_date("no es fecha", "yyyy"), "no es fecha");
}

#[test]
fn validators() {
    let m = json!({});
    let t = |name: &str, args: Value| eval(&m, call(name, args));
    assert_eq!(t("required", json!({"value": ""})), Some(json!(false)));
    assert_eq!(t("required", json!({"value": "x"})), Some(json!(true)));
    assert_eq!(t("required", json!({"value": 0})), Some(json!(true)));
    assert_eq!(
        t("regex", json!({"value": "abc", "pattern": "^a"})),
        Some(json!(true))
    );
    assert_eq!(
        t("regex", json!({"value": "abc", "pattern": "("})),
        Some(json!(false)),
        "regex inválida"
    );
    assert_eq!(
        t("regex", json!({"value": "abc", "pattern": ""})),
        Some(json!(true))
    );
    assert_eq!(t("email", json!({"value": "a@b.co"})), Some(json!(true)));
    assert_eq!(t("email", json!({"value": "a@b"})), Some(json!(false)));
    assert_eq!(
        t("length", json!({"value": "abc", "min": 2, "max": 3})),
        Some(json!(true))
    );
    assert_eq!(
        t("length", json!({"value": "abcd", "max": 3})),
        Some(json!(false))
    );
    assert_eq!(
        t("numeric", json!({"value": "12", "min": 10, "max": 20})),
        Some(json!(true))
    );
    assert_eq!(t("numeric", json!({"value": "x"})), Some(json!(false)));
    assert_eq!(
        t("and", json!({"values": [true, "true"]})),
        Some(json!(true))
    );
    assert_eq!(
        t("and", json!({"values": [true, false]})),
        Some(json!(false))
    );
    assert_eq!(t("or", json!({"values": [false, true]})), Some(json!(true)));
    assert_eq!(t("not", json!({"value": false})), Some(json!(true)));
    assert_eq!(t("nope", json!({})), None);
}

#[test]
fn function_arguments_can_be_paths_and_nested_calls() {
    let model = json!({"form": {"mail": "a@b.co"}});
    let nested = call(
        "not",
        json!({"value": call("email", json!({"value": {"path": "/form/mail"}}))}),
    );
    assert_eq!(eval(&model, nested), Some(json!(false)));
}

// ── checks y acciones ──

#[test]
fn checks_report_the_failing_messages_in_order() {
    let model = json!({"name": ""});
    let ctx = Ctx::new(&model, None);
    let checks = json!([
        {"call": "required", "args": {"value": {"path": "/name"}}, "message": "Requerido"},
        {"call": "length", "args": {"value": {"path": "/name"}, "min": 3}},
        {"call": "required", "args": {"value": "x"}},
    ]);
    assert_eq!(
        run_checks(Some(&checks), &ctx),
        vec!["Requerido", "Validation failed: length"]
    );
    assert!(run_checks(None, &ctx).is_empty());
}

#[test]
fn check_condition_must_be_strictly_true() {
    let model = json!({});
    let ctx = Ctx::new(&model, None);
    let checks = json!([{"condition": {"call": "required", "args": {"value": ""}}, "call": "x", "message": "no"}]);
    assert_eq!(run_checks(Some(&checks), &ctx), vec!["no"]);
}

#[test]
fn action_events_accept_both_shapes() {
    let a = action_event(&json!({"event": {"name": "go", "context": {"k": 1}}})).unwrap();
    assert_eq!(a, ("go".to_string(), json!({"k": 1})));
    let b = action_event(&json!({"name": "go"})).unwrap();
    assert_eq!(b, ("go".to_string(), json!({})));
    assert!(action_event(&json!({"event": {}})).is_none());
}

#[test]
fn action_context_is_resolved_at_click_time() {
    let model = json!({"form": {"name": "Ana"}});
    let scope = json!({"id": 7});
    let ctx = Ctx::new(&model, Some(&scope));
    let out = resolve_context(
        &json!({"name": {"path": "/form/name"}, "row": {"path": "id"}, "src": "form", "nested": {"a": 1}, "up": call("formatNumber", json!({"value": 3}))}),
        &ctx,
    );
    assert_eq!(
        out,
        json!({"name": "Ana", "row": 7, "src": "form", "nested": {"a": 1}, "up": "3"})
    );
}

#[test]
fn action_frame_has_the_wire_shape() {
    let f = action_frame("submit", "s1", "btn", json!({"a": 1}));
    assert_eq!(f["type"], "a2ui:action");
    assert_eq!(f["data"]["name"], "submit");
    assert_eq!(f["data"]["surfaceId"], "s1");
    assert_eq!(f["data"]["sourceComponentId"], "btn");
    assert!(f["data"]["timestamp"].as_str().unwrap().ends_with('Z'));
}
