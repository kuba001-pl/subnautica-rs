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
