# Survival rule implementation status (2026-10-27)

M9s4 serialized swim-charge placement is unresolved after two evidence-based
checks: main scene has zero UpdateSwimCharge behaviours; the SwimChargeFins
prefab hierarchy also has zero. DLL rule and field initializer are readable,
but neither proves an instantiated component or stored rate. Hypothesis:
another spawned hierarchy owns it, or it is unused legacy code. Stop the hunt
here; M9s7's broader census must settle it. The pure charging rule requires an
explicit serialized rate, never silently substitutes the constructor value.
Logs: out/m9s4-real.log (last check output), findings in MODLOG/gameplay.md.

## M9s5 workspace verification blocked by host resources

Two full workspace attempts after M9s5: default parallel compilation failed
with LLVM out-of-memory / Windows paging-file-too-small (1455); a deliberate
CARGO_BUILD_JOBS=1 retry then failed writing sn-client output with no space on
device. Stop retries at this step. Targeted six sn-sim rule tests and installed
quick-slot/Knife timing checks passed. Logs out/m9s5-check-0.log and
out/m9s5-serial-0.log. Complete remaining independent checks, then M9s6;
post-merge workspace verification is still required. Do not call M9s5 complete
unless that full verification eventually passes.
