//! Runtime serialized survival fields, via the existing metadata layout reader.
//! This reads stored fields, never properties or NonSerialized save state.
use crate::{Layouts, ObjectRef, Result};
use sn_unity::{Fields, Value};

pub fn survival_fields(object: &ObjectRef, layouts: &Layouts<'_>, class: &str) -> Result<Fields> {
    let (_, data) = object.data()?;
    let layout = layouts.script("Assembly-CSharp", "", class)?;
    sn_unity::read_script(data, object.file.file().big_endian, &layout)
        .map(|(_, f)| f)
        .map_err(|e| format!("{} {} {class}: {e}", object.file.name, object.path_id))
}
pub(crate) fn number(fields: &Fields, name: &str) -> Result<f64> {
    fields
        .get(name)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("survival field {name}: missing or nonfinite"))
}
/// Stored Oxygen capacity; initial availability is zero from CLR zero-init.
/// The negative-save sentinel is handled separately by restore_oxygen.
pub fn oxygen_capacity(fields: &Fields) -> Result<f64> {
    let n = number(fields, "oxygenCapacity")?;
    if n < 0.0 {
        return Err("Oxygen.oxygenCapacity is negative".into());
    }
    Ok(n)
}
pub fn swim_charge_rate(fields: &Fields) -> Result<f64> {
    let n = number(fields, "chargePerSecond")?;
    if n < 0.0 {
        return Err("UpdateSwimCharge.chargePerSecond is negative".into());
    }
    Ok(n)
}

pub fn tool_timing(fields: &Fields, named_animation: bool) -> Result<sn_sim::survival::ToolTiming> {
    let flag = |n| {
        fields
            .get(n)
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("PlayerTool: missing {n}"))
    };
    Ok(sn_sim::survival::ToolTiming {
        draw: number(fields, "drawTime")? as f32,
        holster: number(fields, "holsterTime")? as f32,
        drop: number(fields, "dropTime")? as f32,
        bash: number(fields, "bashTime")? as f32,
        animated: flag("hasAnimations")?,
        named_animation,
        first_use: flag("hasFirstUseAnimation")?,
    })
}

pub fn knife_info(fields: &Fields) -> Result<sn_sim::survival::KnifeInfo> {
    let damage_type = fields
        .get("damageType")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .ok_or("Knife.damageType missing")?;
    let info = sn_sim::survival::KnifeInfo {
        damage: number(fields, "damage")?,
        reach: number(fields, "attackDist")?,
        damage_type,
    };
    info.validate().map_err(|e| e.to_string())?;
    Ok(info)
}
pub fn cabinet_info(fields: &Fields) -> Result<(sn_sim::survival::CabinetInfo, sn_unity::PPtr)> {
    let info = sn_sim::survival::CabinetInfo {
        interval: number(fields, "medKitSpawnInterval")?,
        start_with_kit: fields
            .get("startWithMedKit")
            .and_then(Value::as_bool)
            .ok_or("MedicalCabinet.startWithMedKit missing")?,
    };
    info.validate().map_err(|e| e.to_string())?;
    let construction = fields
        .get("constructable")
        .and_then(Value::as_pptr)
        .ok_or("MedicalCabinet.constructable missing")?;
    Ok((info, construction))
}
pub fn harvest_info(
    entry: Option<&crate::TechEntry>,
    defaults: &crate::code::TechDefaults,
) -> Result<sn_sim::survival::HarvestInfo> {
    let kind = entry
        .and_then(|e| e.harvest_type)
        .unwrap_or(defaults.harvest_type);
    let output = entry
        .and_then(|e| e.harvest_output)
        .unwrap_or(defaults.harvest_output);
    let bonus = entry
        .and_then(|e| e.harvest_final_cut_bonus)
        .unwrap_or(defaults.harvest_final_cut_bonus);
    Ok(sn_sim::survival::HarvestInfo {
        kind,
        output,
        final_bonus: u32::try_from(bonus).map_err(|_| "negative harvest bonus")?,
    })
}
/// `SpawnStoredLoot`'s stored fields. The curve must be clamped and
/// unweighted (the only kind our evaluator supports); others are errors.
#[derive(Clone, Debug)]
pub struct StoredLootInfo {
    pub min: i32,
    pub max: i32,
    pub distribution: sn_ui::clip::Curve,
}
pub fn stored_loot_info(fields: &Fields) -> Result<StoredLootInfo> {
    let int = |n| {
        fields
            .get(n)
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
            .ok_or_else(|| format!("SpawnStoredLoot.{n} missing"))
    };
    Ok(StoredLootInfo {
        min: int("randomStartMinItems")?,
        max: int("randomStartMaxItems")?,
        distribution: crate::hud::curve_field(fields, "distribution")
            .map_err(|e| format!("SpawnStoredLoot.{e}"))?,
    })
}
