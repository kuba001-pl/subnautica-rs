use super::*;

/// Made-up language entries (not the game's) for every key.
fn strings() -> TooltipStrings {
    let get = |k: &str| -> String {
        match k {
            "FoodFormat" => "F: {0:+#;-#;+0}",
            "WaterFormat" => "W: {0:+#;-#;+0}",
            "HealthFormat" => "H: {0:+#;-#;+0}",
            "DecomposingFormat" => "{0} {1}",
            "BatteryCharge" => "C {0:P0} ({1}/{2})",
            "Ripe" => "ripe",
            "Decomposing" => "old",
            "Rotting" => "rotten",
            "Eat" => "eat it",
            "Drop" => "drop it",
            "SwitchContainer" => "move it",
            "KeyRange15" => "1-5",
            "BindQuickSlot" => "bind it",
            other => other,
        }
        .to_string()
    };
    TooltipStrings::new(&get, ["L".into(), "R".into(), "D".into(), "F".into()])
}

#[test]
fn cs_format_sections_percent_and_braces() {
    let f = |s: &str, a: Arg| cs_format(s, &[a]);
    assert_eq!(f("{0:+#;-#;+0}", Arg::Int(5)), "+5");
    assert_eq!(f("{0:+#;-#;+0}", Arg::Int(-12)), "-12");
    assert_eq!(f("{0:+#;-#;+0}", Arg::Int(0)), "+0");
    // A float rounds half away from zero; one section writes the minus.
    assert_eq!(f("{0:+#;-#;+0}", Arg::Float(49.5)), "+50");
    assert_eq!(f("{0:#}", Arg::Int(-7)), "-7");
    assert_eq!(f("{0:00}", Arg::Int(7)), "07");
    assert_eq!(f("{0:P0}", Arg::Float(0.5)), "50%");
    assert_eq!(f("{0:P0}", Arg::Float(0.996)), "100%");
    assert_eq!(f("{0}", Arg::Float(100.0)), "100");
    assert_eq!(f("{{x}} {0}", Arg::Str("y".into())), "{x} y");
    // Out of range or not in the subset: left as written.
    assert_eq!(f("a {1} b", Arg::Int(1)), "a {1} b");
    assert_eq!(f("{0:#,##0}", Arg::Int(1)), "{0:#,##0}");
    assert_eq!(
        cs_format("{0} {1}", &[Arg::Str("a".into()), Arg::Str("b".into())]),
        "a b"
    );
}

fn plain(title: &str) -> ItemFacts {
    ItemFacts {
        title: title.into(),
        description: format!("about {title}"),
        eatable: None,
        first_aid_kit: false,
        battery: None,
    }
}

const TITLE: &str = "<size=25><color=#ffffffff>";
const DESC: &str = "<size=20><color=#DDDEDEFF>";
const END: &str = "</color></size>";

#[test]
fn item_commons_writes_title_values_then_description() {
    let s = strings();
    assert_eq!(
        item_commons(&s, &plain("Ore"), true),
        format!("{TITLE}Ore{END}\n{DESC}about Ore{END}")
    );
    let mut water = plain("Water");
    water.eatable = Some(Eatable {
        food_value: 0.0,
        water_value: 20.0,
        decomposes: false,
        food_now: 0.0,
        water_now: 19.2,
    });
    // Food 0 is not written; water rounds up.
    assert_eq!(
        item_commons(&s, &water, true),
        format!("{TITLE}Water{END}\n{DESC}W: +20{END}\n{DESC}about Water{END}")
    );
    // Without survival, no values.
    assert_eq!(
        item_commons(&s, &water, false),
        format!("{TITLE}Water{END}\n{DESC}about Water{END}")
    );
    let mut kit = plain("Kit");
    kit.first_aid_kit = true;
    assert_eq!(
        item_commons(&s, &kit, true),
        format!("{TITLE}Kit{END}\n{DESC}H: +50{END}\n{DESC}about Kit{END}")
    );
    // A battery's charge replaces the description.
    let mut cell = plain("Cell");
    cell.battery = Some((75.0, 100.0));
    assert_eq!(
        item_commons(&s, &cell, true),
        format!("{TITLE}Cell{END}\n{DESC}C 75% (75/100){END}")
    );
}

#[test]
fn decomposing_food_gets_its_state_in_the_title() {
    let s = strings();
    let mut fish = plain("Fish");
    let e = |food_now: f32, water_now: f32| Eatable {
        food_value: 10.0,
        water_value: 4.0,
        decomposes: true,
        food_now,
        water_now,
    };
    let title = |f: &ItemFacts| item_commons(&s, f, true).split('\n').next().unwrap().to_string();
    fish.eatable = Some(e(10.0, 4.0));
    assert_eq!(title(&fish), format!("{TITLE}Fish{END}"));
    fish.eatable = Some(e(8.0, 3.0));
    assert_eq!(title(&fish), format!("{TITLE}old Fish{END}"));
    fish.eatable = Some(e(4.0, 1.0));
    assert_eq!(title(&fish), format!("{TITLE}ripe Fish{END}"));
    fish.eatable = Some(e(-1.0, -1.0));
    assert_eq!(title(&fish), format!("{TITLE}rotten Fish{END}"));
    // Negative values are written with their sign.
    let text = item_commons(&s, &fish, true);
    assert!(text.contains("F: -1") && text.contains("W: -1"), "{text}");
}

#[test]
fn item_actions_bind_hint_then_buttons_3_0_2_1() {
    let s = strings();
    let act = |key: &str, word: &str| {
        format!("<size=20><color=#ffffffff>{key}</color> - <color=#00ffffff>{word}</color></size>")
    };
    assert_eq!(item_actions(&s, [0; 4], false), "");
    assert_eq!(item_actions(&s, [0; 4], true), act("1-5", "bind it"));
    assert_eq!(
        item_actions(&s, [EAT, DROP, 0, 0], true),
        [act("1-5", "bind it"), act("L", "eat it"), act("R", "drop it")].join("\n")
    );
    assert_eq!(
        item_actions(&s, [SWITCH, DROP, DROP, EAT], false),
        [
            act("F", "eat it"),
            act("L", "move it"),
            act("D", "drop it"),
            act("R", "drop it")
        ]
        .join("\n")
    );
    let (prefix, postfix) = inventory_item(&s, &plain("Ore"), true, [SWITCH, 0, 0, 0], false);
    assert!(prefix.starts_with(TITLE));
    assert_eq!(postfix, act("L", "move it"));
}
