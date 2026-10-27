//! The survival rules' numbers (M9s), read from the player's
//! `Assembly-CSharp.dll`: the game's compiler inlined most of them into
//! the methods that use them, so each is read from that method's float
//! constants, matched against the whole sequence the method is known to
//! load (an edited method is an error with its IL offset, not a misread),
//! and checked against the `SurvivalConstants` declaration of the same
//! name where one exists. See `docs/formats/gameplay.md` § Survival.

use std::collections::BTreeMap;

use sn_dotnet::Assembly;
use sn_sim::survival::{ConsumeRules, DecayRules, EnergyRules, HungerRules};

use crate::Result;
use crate::code::GAME_ASSEMBLY;

/// One expected float constant of a method, in code order.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Slot {
    /// A named rule value; repeated names must agree.
    Named(&'static str),
    /// A constant that is part of the code's shape, not a rule (a clamp's
    /// 0, `float.Epsilon`): it must have exactly this value.
    Fixed(f32),
}

use Slot::{Fixed, Named};

/// `float.Epsilon`.
const FLOAT_EPSILON: f32 = f32::from_bits(1);

/// Matches a method's float constants against `shape` and returns the
/// named values (with their IL offsets, for provenance).
pub(crate) fn match_literals(
    asm: &Assembly,
    type_name: &str,
    method: &str,
    shape: &[Slot],
) -> Result<BTreeMap<&'static str, (f32, usize)>> {
    let what = format!("{GAME_ASSEMBLY}: {type_name}.{method}");
    let found = sn_dotnet::float_literals(asm, "", type_name, method)
        .map_err(|e| format!("{what}: {e}"))?;
    if found.len() != shape.len() {
        return Err(format!(
            "{what}: {} float constants, expected {} (offsets {:?})",
            found.len(),
            shape.len(),
            found.iter().map(|f| f.0).collect::<Vec<_>>()
        ));
    }
    let mut out: BTreeMap<&'static str, (f32, usize)> = BTreeMap::new();
    for (&(offset, value), slot) in found.iter().zip(shape) {
        match *slot {
            Fixed(v) if value.to_bits() == v.to_bits() => {}
            Fixed(v) => {
                return Err(format!(
                    "{what}: IL offset {offset:#x}: constant {value}, expected {v}"
                ));
            }
            Named(name) => match out.get(name) {
                Some(&(seen, at)) if seen.to_bits() != value.to_bits() => {
                    return Err(format!(
                        "{what}: {name} is {seen} at IL offset {at:#x} but {value} at {offset:#x}"
                    ));
                }
                Some(_) => {}
                None => {
                    out.insert(name, (value, offset));
                }
            },
        }
    }
    Ok(out)
}

/// The value of a named slot (present by construction of the shape).
pub(crate) fn take(map: &BTreeMap<&'static str, (f32, usize)>, name: &str) -> Result<f32> {
    map.get(name)
        .map(|v| v.0)
        .ok_or_else(|| format!("{GAME_ASSEMBLY}: no {name}"))
}

/// Checks a value read from code against its `SurvivalConstants`
/// declaration: the declaration alone does not prove what the method uses
/// (the compiler copied it), but a difference means the reading is wrong.
fn agrees(asm: &Assembly, constant: &str, value: f32) -> Result<()> {
    let declared = sn_dotnet::const_f32(asm, "", "SurvivalConstants", constant)
        .map_err(|e| format!("{GAME_ASSEMBLY}: {e}"))?;
    if declared.to_bits() == value.to_bits() {
        Ok(())
    } else {
        Err(format!(
            "{GAME_ASSEMBLY}: SurvivalConstants.{constant} is {declared}, code uses {value}"
        ))
    }
}

/// [`HungerRules`] from the game's code (M9s1): `Survival`'s constructor
/// (the interval), `UpdateStats` (drain, caps, starvation, warning
/// thresholds), `UpdateHunger` (regeneration), `ResetStats` (start
/// stats) and `OnHealTempDamage`.
pub fn hunger_rules(bytes: &[u8]) -> Result<HungerRules> {
    let e = |e: sn_dotnet::Error| format!("{GAME_ASSEMBLY}: {e}");
    let asm = Assembly::parse(bytes).map_err(e)?;
    let interval = sn_dotnet::field_initializer_f32(&asm, "", "Survival", "kUpdateHungerInterval")
        .map_err(e)?;
    let stats = match_literals(
        &asm,
        "Survival",
        "UpdateStats",
        &[
            Fixed(0.0),
            Fixed(FLOAT_EPSILON),
            Named("food_time"),
            Named("stat_max"),
            Named("starve_damage"),
            Fixed(0.0),
            Named("food_max"),
            Named("water_time"),
            Named("stat_max"),
            Named("starve_damage"),
            Fixed(0.0),
            Named("stat_max"),
            Named("low"),
            Named("critical"),
            Named("low"),
            Named("critical"),
        ],
    )?;
    let hunger = match_literals(
        &asm,
        "Survival",
        "UpdateHunger",
        &[
            Fixed(FLOAT_EPSILON),
            Named("heal_threshold"),
            Named("regen"),
        ],
    )?;
    let temp = match_literals(
        &asm,
        "Survival",
        "OnHealTempDamage",
        &[Named("per_temp"), Fixed(0.0), Named("food_max")],
    )?;
    let (start_food, start_water) = crate::code::survival_start(bytes)?;
    let rules = HungerRules {
        interval: f64::from(interval),
        food_time: f64::from(take(&stats, "food_time")?),
        water_time: f64::from(take(&stats, "water_time")?),
        stat_max: f64::from(take(&stats, "stat_max")?),
        food_max: f64::from(take(&stats, "food_max")?),
        starve_damage: f64::from(take(&stats, "starve_damage")?),
        heal_threshold: f64::from(take(&hunger, "heal_threshold")?),
        regen_per_second: f64::from(take(&hunger, "regen")?),
        start_food: f64::from(start_food),
        start_water: f64::from(start_water),
        low: f64::from(take(&stats, "low")?),
        critical: f64::from(take(&stats, "critical")?),
        food_per_temp_damage: f64::from(take(&temp, "per_temp")?),
    };
    if take(&temp, "food_max")? != take(&stats, "food_max")? {
        return Err(format!(
            "{GAME_ASSEMBLY}: Survival.OnHealTempDamage's food cap differs from UpdateStats'"
        ));
    }
    for (constant, value) in [
        ("kFoodTime", take(&stats, "food_time")?),
        ("kWaterTime", take(&stats, "water_time")?),
        ("kMaxStat", take(&stats, "stat_max")?),
        ("kMaxOverfillStat", take(&stats, "food_max")?),
        ("kStarveDamage", take(&stats, "starve_damage")?),
        ("kLowFoodThreshold", take(&stats, "low")?),
        ("kLowWaterThreshold", take(&stats, "low")?),
        ("kCriticalFoodThreshold", take(&stats, "critical")?),
        ("kCriticalWaterThreshold", take(&stats, "critical")?),
        ("kFoodWaterHealThreshold", take(&hunger, "heal_threshold")?),
        ("kBaseRegenRate", take(&hunger, "regen")?),
        ("kFoodPerTempDamage", take(&temp, "per_temp")?),
        ("kStartFoodStat", start_food),
        ("kStartWaterStat", start_water),
    ] {
        agrees(&asm, constant, value)?;
    }
    rules
        .validate()
        .map_err(|r| format!("{GAME_ASSEMBLY}: hunger rules: {r}"))?;
    Ok(rules)
}

/// A `TechType` by name, from the game's enum, and checked to occur
/// exactly once among `method`'s integer constants (the method compares
/// with it).
fn tech_in(asm: &Assembly, name: &str, type_name: &str, method: &str) -> Result<i32> {
    let what = format!("{GAME_ASSEMBLY}: {type_name}.{method}");
    let value = sn_dotnet::enum_values(asm, "", "TechType")
        .map_err(|e| format!("{GAME_ASSEMBLY}: {e}"))?
        .into_iter()
        .find(|(n, _)| n == name)
        .and_then(|(_, v)| i32::try_from(v).ok())
        .ok_or_else(|| format!("{GAME_ASSEMBLY}: no TechType.{name}"))?;
    let ints =
        sn_dotnet::int_literals(asm, "", type_name, method).map_err(|e| format!("{what}: {e}"))?;
    match ints.iter().filter(|i| i.1 == value).count() {
        1 => Ok(value),
        n => Err(format!(
            "{what}: TechType.{name} ({value}) occurs {n} times, expected 1"
        )),
    }
}

/// [`ConsumeRules`] from the game's code (M9s2): `Survival.Eat` (the
/// food gate, the vitals notification's threshold, the oxygen food's
/// oxygen; its caps must equal [`hunger_rules`]'), `Survival.Use` (the
/// first aid kit's health and success threshold), `Eatable`'s value
/// getters (the decay floor, the same in both) and the two tech types.
pub fn consume_rules(bytes: &[u8]) -> Result<ConsumeRules> {
    let hunger = hunger_rules(bytes)?;
    let asm = Assembly::parse(bytes).map_err(|e| format!("{GAME_ASSEMBLY}: {e}"))?;
    let eat = match_literals(
        &asm,
        "Survival",
        "Eat",
        &[
            Fixed(0.0),
            Named("food_gate"),
            Fixed(0.0),
            Named("food_max"),
            Fixed(0.0),
            Fixed(0.0),
            Fixed(0.0),
            Named("stat_max"),
            Fixed(0.0),
            Named("vitals_ok"),
            Named("vitals_ok"),
            Named("vitals_ok"),
            Named("vitals_ok"),
            Named("volume"),
            Named("oxygen"),
        ],
    )?;
    if f64::from(take(&eat, "food_max")?) != hunger.food_max
        || f64::from(take(&eat, "stat_max")?) != hunger.stat_max
    {
        return Err(format!(
            "{GAME_ASSEMBLY}: Survival.Eat's caps differ from UpdateStats'"
        ));
    }
    let used = match_literals(
        &asm,
        "Survival",
        "Use",
        &[
            Named("first_aid"),
            Named("success"),
            Named("sound_distance"),
            Named("volume"),
        ],
    )?;
    let mut floor = None;
    for method in ["GetFoodValue", "GetWaterValue"] {
        let v = take(
            &match_literals(&asm, "Eatable", method, &[Named("floor")])?,
            "floor",
        )?;
        if floor.is_some_and(|f| f != v) {
            return Err(format!(
                "{GAME_ASSEMBLY}: Eatable's food and water floors differ"
            ));
        }
        floor = Some(v);
    }
    agrees(&asm, "kBladderFishO2OnEat", take(&eat, "oxygen")?)?;
    agrees(&asm, "kFirstAidKitHealth", take(&used, "first_aid")?)?;
    let rules = ConsumeRules {
        food_gate: f64::from(take(&eat, "food_gate")?),
        vitals_ok: f64::from(take(&eat, "vitals_ok")?),
        first_aid: tech_in(&asm, "FirstAidKit", "Survival", "Use")?,
        first_aid_health: f64::from(take(&used, "first_aid")?),
        heal_success: f64::from(take(&used, "success")?),
        oxygen_food: tech_in(&asm, "Bladderfish", "Survival", "Eat")?,
        oxygen_on_eat: f64::from(take(&eat, "oxygen")?),
        decay_floor: f64::from(floor.unwrap_or(f32::NAN)),
    };
    rules
        .validate()
        .map_err(|r| format!("{GAME_ASSEMBLY}: consume rules: {r}"))?;
    Ok(rules)
}

/// [`DecayRules`] from the game's code (M9s3): `Eatable`'s constructor
/// (`despawnDelay`), `GetFoodValue`/`GetWaterValue` (the decay floor),
/// `GetSecondaryTooltip` (the ripe and decomposing fractions),
/// `IsRotten`, `SetDecomposes` and `StartDespawnInvoke` (the despawn
/// check interval).
pub fn decay_rules(bytes: &[u8]) -> Result<DecayRules> {
    let e = |e: sn_dotnet::Error| format!("{GAME_ASSEMBLY}: {e}");
    let asm = Assembly::parse(bytes).map_err(e)?;
    let delay = sn_dotnet::field_initializer_f32(&asm, "", "Eatable", "despawnDelay").map_err(e)?;
    let mut floor = None;
    for method in ["GetFoodValue", "GetWaterValue"] {
        let v = take(
            &match_literals(&asm, "Eatable", method, &[Named("floor")])?,
            "floor",
        )?;
        if floor.is_some_and(|f| f != v) {
            return Err(format!(
                "{GAME_ASSEMBLY}: Eatable's food and water floors differ"
            ));
        }
        floor = Some(v);
    }
    let tooltip = match_literals(
        &asm,
        "Eatable",
        "GetSecondaryTooltip",
        &[Fixed(0.0), Fixed(0.0), Named("ripe"), Named("decomposing")],
    )?;
    match_literals(&asm, "Eatable", "IsRotten", &[Fixed(0.0), Fixed(0.0)])?;
    match_literals(&asm, "Eatable", "SetDecomposes", &[Fixed(0.0), Fixed(0.0)])?;
    let despawn = match_literals(
        &asm,
        "Eatable",
        "StartDespawnInvoke",
        &[Fixed(0.0), Named("interval"), Named("interval")],
    )?;
    let rules = DecayRules {
        decay_floor: f64::from(floor.unwrap_or(f32::NAN)),
        ripe_fraction: f64::from(take(&tooltip, "ripe")?),
        decomposing_fraction: f64::from(take(&tooltip, "decomposing")?),
        despawn_check_interval: f64::from(take(&despawn, "interval")?),
        default_despawn_delay: f64::from(delay),
    };
    rules
        .validate()
        .map_err(|r| format!("{GAME_ASSEMBLY}: decay rules: {r}"))?;
    Ok(rules)
}

/// [`EnergyRules`] from the game's code (M9s3): `Battery`'s constructor
/// (`_capacity`), `EnergyMixin`'s constructor (`defaultBattery`,
/// `defaultBatteryCharge`, `maxEnergy`), `get_charge` (the recovery ramp)
/// and `ModifyCharge`.
pub fn energy_rules(bytes: &[u8]) -> Result<EnergyRules> {
    let e = |e: sn_dotnet::Error| format!("{GAME_ASSEMBLY}: {e}");
    let asm = Assembly::parse(bytes).map_err(e)?;
    let capacity = sn_dotnet::field_initializer_f32(&asm, "", "Battery", "_capacity").map_err(e)?;
    let default_charge =
        sn_dotnet::field_initializer_f32(&asm, "", "EnergyMixin", "defaultBatteryCharge")
            .map_err(e)?;
    let max_energy =
        sn_dotnet::field_initializer_f32(&asm, "", "EnergyMixin", "maxEnergy").map_err(e)?;
    if capacity != max_energy {
        return Err(format!(
            "{GAME_ASSEMBLY}: Battery._capacity ({capacity}) != EnergyMixin.maxEnergy ({max_energy})"
        ));
    }
    let default_tech = tech_in(&asm, "Battery", "EnergyMixin", ".ctor")?;
    let charge_getter = match_literals(
        &asm,
        "EnergyMixin",
        "get_charge",
        &[Fixed(0.0), Named("recovery"), Named("recovery")],
    )?;
    match_literals(
        &asm,
        "EnergyMixin",
        "ModifyCharge",
        &[
            Fixed(0.0),
            Fixed(0.0),
            Fixed(0.0),
            Fixed(0.0),
            Fixed(0.0),
            Fixed(0.0),
        ],
    )?;
    let rules = EnergyRules {
        default_battery_tech: default_tech,
        default_battery_charge: default_charge,
        default_battery_capacity: capacity,
        recovery_time: take(&charge_getter, "recovery")?,
    };
    rules
        .validate()
        .map_err(|r| format!("{GAME_ASSEMBLY}: energy rules: {r}"))?;
    Ok(rules)
}

/// Code-only equipment coefficients, read from their bounded IL literal shapes.
pub fn equipment_rules(bytes: &[u8]) -> Result<sn_sim::survival::EquipmentRules> {
    let asm = Assembly::parse(bytes).map_err(|e| format!("{GAME_ASSEMBLY}: {e}"))?;
    let m = match_literals(
        &asm,
        "UnderwaterMotor",
        "AlterMaxSpeed",
        &[
            Named("plasteel"),
            Named("tank"),
            Named("double"),
            Named("heavy"),
            Named("spare"),
            Named("min"),
            Named("min"),
            Named("min"),
            Named("suit"),
            Named("fins"),
            Named("glide"),
            Named("hand"),
            Named("surface"),
            Named("transition"),
        ],
    )?;
    let charge = match_literals(
        &asm,
        "UpdateSwimCharge",
        "FixedUpdate",
        &[Named("threshold")],
    )?;
    let tech = |name: &str| enum_value(&asm, "TechType", name);
    let r = sn_sim::survival::EquipmentRules {
        tank_penalties: [
            ("PlasteelTank", "plasteel"),
            ("Tank", "tank"),
            ("DoubleTank", "double"),
            ("HighCapacityTank", "heavy"),
        ]
        .into_iter()
        .map(|(t, n)| Ok((tech(t)?, f64::from(take(&m, n)?))))
        .collect::<Result<_>>()?,
        heavy_tank: tech("HighCapacityTank")?,
        spare_penalty: f64::from(take(&m, "spare")?),
        minimum_speed: f64::from(take(&m, "min")?),
        reinforced_suit: tech("ReinforcedDiveSuit")?,
        suit_penalty: f64::from(take(&m, "suit")?),
        fins_bonuses: [("Fins", "fins"), ("UltraGlideFins", "glide")]
            .into_iter()
            .map(|(t, n)| Ok((tech(t)?, f64::from(take(&m, n)?))))
            .collect::<Result<_>>()?,
        held_penalty: f64::from(take(&m, "hand")?),
        surface_multiplier: f64::from(take(&m, "surface")?),
        modifier_transition: f64::from(take(&m, "transition")?),
        surface_fill: f64::from(
            sn_dotnet::field_initializer_f32(
                &asm,
                "",
                "OxygenManager",
                "oxygenUnitsPerSecondSurface",
            )
            .map_err(|e| format!("{GAME_ASSEMBLY}: {e}"))?,
        ),
        charge_speed_threshold: f64::from(take(&charge, "threshold")?),
    };
    r.validate().map_err(|e| format!("{GAME_ASSEMBLY}: {e}"))?;
    Ok(r)
}

pub(crate) fn enum_value(asm: &Assembly, ty: &str, name: &str) -> Result<i32> {
    sn_dotnet::enum_values(asm, "", ty)
        .map_err(|e| format!("{GAME_ASSEMBLY}: {e}"))?
        .into_iter()
        .find(|(n, _)| n == name)
        .and_then(|(_, v)| i32::try_from(v).ok())
        .ok_or_else(|| format!("{GAME_ASSEMBLY}: missing {ty}.{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_dotnet::encode::{Builder, Il};

    const LITERAL: u16 = 0x0056;
    const VALUE_FIELD: u16 = 0x0606;
    const LDARG_0: u8 = 0x02;
    const POP: u8 = 0x26;

    type Method = (&'static str, &'static str, Vec<f32>, Vec<i32>);

    /// Our own synthetic numbers, in each method's order.
    fn methods() -> Vec<Method> {
        vec![
            (
                "Survival",
                "UpdateStats",
                vec![
                    0.0,
                    FLOAT_EPSILON,
                    300.0,
                    10.0,
                    2.0,
                    0.0,
                    30.0,
                    200.0,
                    10.0,
                    2.0,
                    0.0,
                    10.0,
                    5.0,
                    1.0,
                    5.0,
                    1.0,
                ],
                vec![],
            ),
            (
                "Survival",
                "UpdateHunger",
                vec![FLOAT_EPSILON, 20.0, 0.5],
                vec![],
            ),
            (
                "Survival",
                "OnHealTempDamage",
                vec![0.125, 0.0, 30.0],
                vec![],
            ),
            (
                "Survival",
                "Eat",
                vec![
                    0.0, 9.0, 0.0, 30.0, 0.0, 0.0, 0.0, 10.0, 0.0, 4.0, 4.0, 4.0, 4.0, 1.0, 3.0,
                ],
                vec![0, 1, 1, 7002],
            ),
            (
                "Survival",
                "Use",
                vec![12.0, 0.5, 20.0, 1.0],
                vec![0, 7001, 1],
            ),
            ("Eatable", "GetFoodValue", vec![-3.0], vec![]),
            ("Eatable", "GetWaterValue", vec![-3.0], vec![]),
            (
                "Eatable",
                "GetSecondaryTooltip",
                vec![0.0, 0.0, 0.4, 0.8],
                vec![],
            ),
            ("Eatable", "IsRotten", vec![0.0, 0.0], vec![]),
            ("Eatable", "SetDecomposes", vec![0.0, 0.0], vec![]),
            ("Eatable", "StartDespawnInvoke", vec![0.0, 6.0, 6.0], vec![]),
            ("Battery", "Dummy", vec![], vec![]),
            ("EnergyMixin", "get_charge", vec![0.0, 2.5, 2.5], vec![]),
            (
                "EnergyMixin",
                "ModifyCharge",
                vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                vec![],
            ),
        ]
    }

    /// A synthetic assembly shaped like the game's survival code; `edit`
    /// may change any method's float constants first.
    fn assembly_with(edit: &dyn Fn(&str, &mut Vec<f32>)) -> Vec<u8> {
        let mut b = Builder::new();
        type MethodSpec<'a> = (&'a str, Vec<f32>, Vec<i32>);
        let mut by_type: Vec<(&str, Vec<MethodSpec>)> = Vec::new();
        for (ty, m, mut floats, ints) in methods() {
            edit(m, &mut floats);
            match by_type.iter_mut().find(|t| t.0 == ty) {
                Some(t) => t.1.push((m, floats, ints)),
                None => by_type.push((ty, vec![(m, floats, ints)])),
            }
        }
        for (ty, list) in by_type {
            b.type_def("", ty);
            if ty == "Eatable" {
                let delay = b.field(0x0001, "despawnDelay");
                let ctor = Il::default().op(LDARG_0).ldc_r4(120.0).stfld(delay).ret();
                b.method(".ctor", 0, Some(&ctor.0));
            }
            if ty == "Battery" {
                let cap = b.field(0x0001, "_capacity");
                let ctor = Il::default().op(LDARG_0).ldc_r4(80.0).stfld(cap).ret();
                b.method(".ctor", 0, Some(&ctor.0));
            }
            if ty == "EnergyMixin" {
                let def_bat = b.field(0x0001, "defaultBattery");
                let def_chg = b.field(0x0001, "defaultBatteryCharge");
                let max_e = b.field(0x0001, "maxEnergy");
                let ctor = Il::default()
                    .op(LDARG_0)
                    .ldc(504)
                    .stfld(def_bat)
                    .op(LDARG_0)
                    .ldc_r4(0.75)
                    .stfld(def_chg)
                    .op(LDARG_0)
                    .ldc_r4(80.0)
                    .stfld(max_e)
                    .ret();
                b.method(".ctor", 0, Some(&ctor.0));
            }
            if ty == "Survival" {
                let food = b.field(0x0001, "food");
                let water = b.field(0x0001, "water");
                let interval = b.field(0x0001, "kUpdateHungerInterval");
                let ctor = Il::default().op(LDARG_0).ldc_r4(4.0).stfld(interval).ret();
                b.method(".ctor", 0, Some(&ctor.0));
                let reset = Il::default()
                    .op(LDARG_0)
                    .ldc_r4(6.0)
                    .stfld(food)
                    .op(LDARG_0)
                    .ldc_r4(8.0)
                    .stfld(water)
                    .ret();
                b.method("ResetStats", 0, Some(&reset.0));
            }
            for (m, floats, ints) in list {
                let mut code = Il::default();
                for v in floats {
                    code = code.ldc_r4(v).op(POP);
                }
                for v in ints {
                    code = code.ldc(v).op(POP);
                }
                b.method(m, 1, Some(&code.ret().0));
            }
        }
        b.type_def("", "TechType");
        b.field(VALUE_FIELD, "value__");
        for (name, v) in [
            ("Battery", 504),
            ("FirstAidKit", 7001),
            ("Bladderfish", 7002),
        ] {
            let f = b.field(LITERAL, name);
            b.constant_i4(f, v);
        }
        b.type_def("", "SurvivalConstants");
        for (name, v) in [
            ("kFoodTime", 300.0),
            ("kWaterTime", 200.0),
            ("kMaxStat", 10.0),
            ("kMaxOverfillStat", 30.0),
            ("kStarveDamage", 2.0),
            ("kLowFoodThreshold", 5.0),
            ("kLowWaterThreshold", 5.0),
            ("kCriticalFoodThreshold", 1.0),
            ("kCriticalWaterThreshold", 1.0),
            ("kFoodWaterHealThreshold", 20.0),
            ("kBaseRegenRate", 0.5),
            ("kFoodPerTempDamage", 0.125),
            ("kStartFoodStat", 6.0),
            ("kStartWaterStat", 8.0),
            ("kBladderFishO2OnEat", 3.0),
            ("kFirstAidKitHealth", 12.0),
        ] {
            let f = b.field(LITERAL, name);
            b.constant_r4(f, v);
        }
        b.build()
    }

    /// [`assembly_with`] with `UpdateStats`' constant `change.0` set to
    /// `change.1`, and one more constant when `extra`.
    fn assembly(change: Option<(usize, f32)>, extra: bool) -> Vec<u8> {
        assembly_with(&|m, floats| {
            if m == "UpdateStats" {
                if let Some((i, v)) = change {
                    floats[i] = v;
                }
                if extra {
                    floats.push(7.0);
                }
            }
        })
    }

    /// [`assembly_with`] with float `i` of `method` set to `v`.
    fn changed(method: &'static str, i: usize, v: f32) -> Vec<u8> {
        assembly_with(&move |m, floats| {
            if m == method {
                floats[i] = v;
            }
        })
    }

    #[test]
    fn reads_the_rules_from_their_methods() {
        let r = hunger_rules(&assembly(None, false)).unwrap();
        assert_eq!(r.interval, 4.0);
        assert_eq!((r.food_time, r.water_time), (300.0, 200.0));
        assert_eq!((r.stat_max, r.food_max, r.starve_damage), (10.0, 30.0, 2.0));
        assert_eq!((r.low, r.critical), (5.0, 1.0));
        assert_eq!((r.heal_threshold, r.regen_per_second), (20.0, 0.5));
        assert_eq!((r.start_food, r.start_water), (6.0, 8.0));
        assert_eq!(r.food_per_temp_damage, 0.125);
    }

    #[test]
    fn changed_methods_are_errors_with_offsets() {
        // A repeated value that disagrees.
        let e = hunger_rules(&assembly(Some((8, 11.0)), false)).unwrap_err();
        assert!(e.contains("stat_max") && e.contains("IL offset"), "{e}");
        // A fixed part of the shape that changed.
        let e = hunger_rules(&assembly(Some((5, 1.0)), false)).unwrap_err();
        assert!(e.contains("IL offset") && e.contains("expected 0"), "{e}");
        // One constant more.
        let e = hunger_rules(&assembly(None, true)).unwrap_err();
        assert!(e.contains("17 float constants, expected 16"), "{e}");
        // Agrees with the code but not with the declaration.
        let e = hunger_rules(&assembly(Some((2, 301.0)), false)).unwrap_err();
        assert!(e.contains("kFoodTime"), "{e}");
    }

    #[test]
    fn reads_the_consume_rules() {
        let c = consume_rules(&assembly(None, false)).unwrap();
        assert_eq!((c.food_gate, c.vitals_ok), (9.0, 4.0));
        assert_eq!(
            (c.first_aid, c.first_aid_health, c.heal_success),
            (7001, 12.0, 0.5)
        );
        assert_eq!((c.oxygen_food, c.oxygen_on_eat), (7002, 3.0));
        assert_eq!(c.decay_floor, -3.0);
        // Floors that differ, a cap that differs from UpdateStats', a
        // threshold used twice with two values, a declaration that
        // differs: errors.
        let e = consume_rules(&changed("GetWaterValue", 0, -4.0)).unwrap_err();
        assert!(e.contains("floors differ"), "{e}");
        let e = consume_rules(&changed("Eat", 3, 31.0)).unwrap_err();
        assert!(e.contains("caps differ"), "{e}");
        let e = consume_rules(&changed("Eat", 11, 5.0)).unwrap_err();
        assert!(e.contains("vitals_ok") && e.contains("IL offset"), "{e}");
        let e = consume_rules(&changed("Use", 0, 13.0)).unwrap_err();
        assert!(e.contains("kFirstAidKitHealth"), "{e}");
    }

    #[test]
    fn reads_the_decay_and_energy_rules() {
        let bytes = assembly(None, false);
        let d = decay_rules(&bytes).unwrap();
        assert_eq!(d.decay_floor, -3.0);
        assert_eq!(
            (d.ripe_fraction, d.decomposing_fraction),
            (f64::from(0.4f32), f64::from(0.8f32))
        );
        assert_eq!(
            (d.despawn_check_interval, d.default_despawn_delay),
            (6.0, 120.0)
        );
        let e = energy_rules(&bytes).unwrap();
        assert_eq!(e.default_battery_tech, 504);
        assert_eq!(
            (
                e.default_battery_charge,
                e.default_battery_capacity,
                e.recovery_time
            ),
            (0.75, 80.0, 2.5)
        );
        let err = decay_rules(&changed("StartDespawnInvoke", 2, 7.0)).unwrap_err();
        assert!(
            err.contains("interval") && err.contains("IL offset"),
            "{err}"
        );
        let err = decay_rules(&changed("GetSecondaryTooltip", 2, 0.9)).unwrap_err();
        assert!(err.contains("decay rules"), "{err}");
        let err = energy_rules(&changed("get_charge", 2, 3.0)).unwrap_err();
        assert!(
            err.contains("recovery") && err.contains("IL offset"),
            "{err}"
        );
        let err = energy_rules(&changed("ModifyCharge", 3, 1.0)).unwrap_err();
        assert!(
            err.contains("expected 0") && err.contains("IL offset"),
            "{err}"
        );
    }

    #[test]
    fn damaged_assemblies_never_panic() {
        let bytes = assembly(None, false);
        let read = |b: &[u8]| {
            let _ = hunger_rules(b);
            let _ = consume_rules(b);
            let _ = decay_rules(b);
            let _ = energy_rules(b);
        };
        for n in 0..bytes.len() {
            read(&bytes[..n]);
        }
        for i in 0..bytes.len() {
            let mut b = bytes.clone();
            b[i] ^= 0xFF;
            read(&b);
        }
    }
}

#[cfg(test)]
mod equipment_reader_tests {
    use super::*;
    use sn_dotnet::encode::{Builder, Il};
    fn fixture(bad: bool) -> Vec<u8> {
        let mut b = Builder::new();
        b.type_def("", "UnderwaterMotor");
        let mut il = Il::default();
        for x in [
            0.1,
            0.2,
            0.3,
            0.4,
            0.5,
            2.0,
            if bad { 3.0 } else { 2.0 },
            2.0,
            0.6,
            1.1,
            1.2,
            0.7,
            1.4,
            0.8,
        ] {
            il = il.ldc_r4(x).op(0x26);
        }
        b.method("AlterMaxSpeed", 1, Some(&il.ret().0));
        b.type_def("", "UpdateSwimCharge");
        b.method(
            "FixedUpdate",
            1,
            Some(&Il::default().ldc_r4(3.0).op(0x26).ret().0),
        );
        b.type_def("", "OxygenManager");
        let field = b.field(1, "oxygenUnitsPerSecondSurface");
        b.method(
            ".ctor",
            0,
            Some(&Il::default().op(2).ldc_r4(9.0).stfld(field).ret().0),
        );
        b.type_def("", "TechType");
        b.field(0x0606, "value__");
        for (i, name) in [
            "PlasteelTank",
            "Tank",
            "DoubleTank",
            "HighCapacityTank",
            "ReinforcedDiveSuit",
            "Fins",
            "UltraGlideFins",
        ]
        .into_iter()
        .enumerate()
        {
            let f = b.field(0x0056, name);
            b.constant_i4(f, i as i32 + 80);
        }
        b.build()
    }
    #[test]
    fn equipment_coefficients_are_runtime_inputs_and_ambiguity_is_an_offset_error() {
        let data = fixture(false);
        let r = equipment_rules(&data).unwrap();
        assert_eq!(r.charge_speed_threshold, 3.0);
        assert_eq!(r.surface_fill, 9.0);
        assert_eq!(r.tank_penalties[0], (80, f64::from(0.1f32)));
        let error = equipment_rules(&fixture(true)).unwrap_err();
        assert!(error.contains("IL offset"), "{error}");
        for n in (0..data.len()).step_by(47) {
            assert!(equipment_rules(&data[..n]).is_err());
        }
    }
}

/// QuickSlots' numeric rules, including Inventory.Awake's adjacent
/// ldc.i4 -> newobj QuickSlots -> call Inventory.set_quickSlots pattern.
pub fn quick_rules(bytes: &[u8]) -> Result<sn_sim::survival::QuickRules> {
    let asm = Assembly::parse(bytes).map_err(|e| format!("{GAME_ASSEMBLY}: {e}"))?;
    let slots = quick_slot_count(&asm)?;
    let transition = match_literals(
        &asm,
        "QuickSlots",
        "GetTransitionTime",
        &[
            Named("draw"),
            Named("holster"),
            Named("drop"),
            Fixed(0.0),
            Named("max"),
            Fixed(0.0),
            Named("max"),
            Fixed(0.0),
            Named("max"),
            Fixed(0.0),
        ],
    )?;
    let bash = match_literals(
        &asm,
        "QuickSlots",
        "PlayerIsBashing",
        &[Named("bash"), Fixed(0.0)],
    )?;
    let r = sn_sim::survival::QuickRules {
        slots,
        hand: enum_value(&asm, "EquipmentType", "Hand")?,
        scrap: enum_value(&asm, "TechType", "ScrapMetal")?,
        selectable: enum_value(&asm, "QuickSlotType", "Selectable")?,
        selectable_chargeable: enum_value(&asm, "QuickSlotType", "SelectableChargeable")?,
        defaults: sn_sim::survival::ToolTiming {
            draw: take(&transition, "draw")?,
            holster: take(&transition, "holster")?,
            drop: take(&transition, "drop")?,
            bash: take(&bash, "bash")?,
            animated: false,
            named_animation: false,
            first_use: false,
        },
        transition_max: take(&transition, "max")?,
    };
    r.validate().map_err(|e| format!("{GAME_ASSEMBLY}: {e}"))?;
    Ok(r)
}
fn quick_slot_count(asm: &Assembly) -> Result<usize> {
    let e = |e: sn_dotnet::Error| format!("{GAME_ASSEMBLY}: {e}");
    let ty = asm
        .find_type("", "Inventory")
        .map_err(e)?
        .ok_or("Inventory missing")?;
    let method = asm
        .find_method(ty, "Awake")
        .map_err(e)?
        .ok_or("Inventory.Awake missing")?;
    let (body, base) = asm
        .method_body(method)
        .map_err(e)?
        .ok_or("Inventory.Awake body missing")?;
    let code = sn_dotnet::decode(body, base).map_err(e)?;
    let mut counts = Vec::new();
    for w in code.windows(3) {
        if w[1].opcode != 0x73 || w[2].opcode != 0x28 {
            continue;
        }
        let ctor = asm
            .method_token(w[1].token().ok_or("newobj token missing")?)
            .map_err(e)?;
        let setter = asm
            .method_token(w[2].token().ok_or("setter token missing")?)
            .map_err(e)?;
        if ctor.type_name == "QuickSlots"
            && ctor.name == ".ctor"
            && ctor.params == 6
            && setter.type_name == "Inventory"
            && setter.name == "set_quickSlots"
            && setter.params == 1
        {
            let n = w[0]
                .ldc_i4()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n > 0 && *n <= 1024)
                .ok_or_else(|| {
                    format!(
                        "Inventory.Awake: invalid slot count at IL offset {:#x}",
                        w[0].offset
                    )
                })?;
            counts.push(n);
        }
    }
    match counts.as_slice() {
        [n] => Ok(*n),
        _ => Err(format!(
            "Inventory.Awake at IL offset {base:#x}: {} slot-count patterns, expected 1",
            counts.len()
        )),
    }
}

#[cfg(test)]
mod quick_reader_tests {
    use super::*;
    use sn_dotnet::{
        Table,
        encode::{Builder, Il},
    };
    fn fixture(counts: &[i32]) -> Vec<u8> {
        let mut b = Builder::new();
        let quick = b.type_ref("", "QuickSlots");
        let ctor = b.member_ref((Table::TYPE_REF, quick), ".ctor", Some(6));
        b.type_def("", "Inventory");
        let setter = b.method("set_quickSlots", 1, None);
        let mut il = Il::default();
        for n in counts {
            il = il.ldc(*n).newobj(ctor).call(setter);
        }
        b.method("Awake", 0, Some(&il.ret().0));
        b.build()
    }
    #[test]
    fn slot_count_pattern_is_bounded_and_unique() {
        let bytes = fixture(&[7]);
        let asm = Assembly::parse(&bytes).unwrap();
        assert_eq!(quick_slot_count(&asm).unwrap(), 7);
        for counts in [&[][..], &[0][..], &[7, 8][..]] {
            let bytes = fixture(counts);
            let asm = Assembly::parse(&bytes).unwrap();
            assert!(quick_slot_count(&asm).unwrap_err().contains("IL offset"));
        }
    }
}
