# Survival rule implementation status (2026-10-27)

M9s4 serialized swim-charge placement is unresolved after two evidence-based
checks: main scene has zero UpdateSwimCharge behaviours; the SwimChargeFins
prefab hierarchy also has zero. DLL rule and field initializer are readable,
but neither proves an instantiated component or stored rate. Hypothesis:
another spawned hierarchy owns it, or it is unused legacy code. Stop the hunt
here; M9s7's broader census must settle it. The pure charging rule requires an
explicit serialized rate, never silently substitutes the constructor value.
Logs: out/m9s4-real.log (last check output), findings in MODLOG/gameplay.md.

## M9s5 workspace verification (resolved)

The two earlier attempts failed on host resources (out of memory, then no
disk space). Resolved 2026-10-27 with M9s6: `cargo test --workspace` passed
(796/0 failed/70 ignored), clippy and fmt clean (out/m9s6-workspace.log).
