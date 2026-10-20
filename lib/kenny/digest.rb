# frozen_string_literal: true
# Kenny: Digest is implemented natively by the runtime (crates/kenny-runtime/src/digest.rs);
# this replaces the stdlib digest.rb, which wraps the digest C extension.
require "digest.so"
