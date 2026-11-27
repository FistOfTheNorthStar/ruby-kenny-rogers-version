# grass_compiler 0.13.4, with source maps

[grass](https://github.com/connorskees/grass) (MIT, see LICENSE) as published on crates.io,
patched in through `[patch.crates-io]` in the workspace's Cargo.toml. The first commit that
adds this directory has it unchanged; what Kenny adds on top:

- Source maps. The CSS tree keeps where each rule, at-rule, declaration and plain import was
  written (a `span` on `CssStmt::RuleSet`, `Style`, `MediaRule`, `SupportsRule`,
  `UnknownAtRule`, `KeyframesRuleSet` and `CssStmt::Import`). A rule that grass copies into
  `@media`, `@supports` or another at-rule (declarations written directly inside one) maps
  to the rule it was copied from. The serializer records an output offset for each selector,
  property, value, at-rule and comment it writes, and `from_path_with_source_map` returns them as a
  `SourceMap` (`src/source_map.rs`): the sources (the entry file first, then the files used,
  in load order) and the mappings, with `encode_mappings` for the source map format.
  `Visitor::files` lists the files loaded, so a position is looked up without codemap's
  `look_up_pos`, which panics on a position outside every file.
- `@supports` rules' span starts at the `@`, as other at-rules' do (it was the body's span),
  so their mapping, and an error about one, points at the rule.

Kenny turns the map into sassc's JSON, URL comment and embedded map in
crates/kenny-runtime/src/assets.rs (`SourceMapOpts`).

To move to a newer grass: apply these changes to it (`git diff` from the vendoring commit).
