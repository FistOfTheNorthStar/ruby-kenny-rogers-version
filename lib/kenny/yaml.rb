# frozen_string_literal: true
# Kenny: YAML is Psych, implemented natively (crates/kenny-runtime/src/yaml.rs).
require "psych"

YAML = Psych unless defined?(YAML)
