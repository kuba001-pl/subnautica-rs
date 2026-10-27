# Survival rule implementation status (2026-10-27)

M9s4 serialized swim-charge placement is unresolved after two evidence-based
checks: main scene has zero UpdateSwimCharge behaviours; the SwimChargeFins
prefab hierarchy also has zero. DLL rule and field initializer are readable,
but neither proves an instantiated component or stored rate. Hypothesis:
another spawned hierarchy owns it, or it is unused legacy code. Stop the hunt
here; M9s7's broader census must settle it. The pure charging rule requires an
explicit serialized rate, never silently substitutes the constructor value.
Logs: out/m9s4-real.log (last check output), findings in MODLOG/gameplay.md.
