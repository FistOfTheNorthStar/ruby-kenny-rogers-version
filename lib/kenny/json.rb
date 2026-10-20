# frozen_string_literal: true
# Kenny: JSON is implemented natively by the runtime (crates/kenny-runtime/src/json.rs);
# this replaces the stdlib json.rb, which loads the json C extension.
require "json/ext/parser.so"
