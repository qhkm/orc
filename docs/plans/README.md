# Planning notes

These are the working notes from building orc, kept as written. orc is an early experiment, started in February 2026 when vibe coding — building software by directing AI coding agents — had just gone viral, and much of it was built that way: a person setting direction, agents writing and testing code against these plans.

They are a record, not documentation. Some describe designs that changed later; the code and the top-level README are the current truth.

| Date | Note |
|---|---|
| 2026-02-20 | `orc-rust-rewrite-design`, `orc-rust-implementation`: moving from the Go prototype to Rust; session parsers ported from [cli-continues](https://github.com/yigitkonur/cli-continues) |
| 2026-02-20 | `ux-improvements`, `repl-smart-routing-design`, `repl-smart-routing-plan`: TUI polish and the REPL with classifier-based routing |
| 2026-11-08 | `orc-positioning`: what orc is for, compared honestly with [herdr](https://herdr.dev/), and the build order verify → hand off → route |
| 2026-11-08 | `verify-step-plan`, `handoff-step-plan`, `route-step-plan`: the three steps of that loop, each with what was verified live |
