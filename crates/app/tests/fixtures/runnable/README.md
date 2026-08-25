# runnable

A minimal project fixture for `e2e_run_and_stop_shows_console_output`: no
`Cargo.toml`/`package.json`/`Makefile`, so `run_core::detect` finds nothing
here and the run console's only configuration is the one already seeded in
`.ide/settings.toml` — a `/bin/sh` command chosen for the flow's own sake
(fast, no compiler, keeps running until stopped) rather than any detected
build tool.
