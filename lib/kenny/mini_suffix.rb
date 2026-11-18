# frozen_string_literal: true

# The mini_suffix gem's API on the public_suffix gem. mini_suffix calls libpsl through FFI (when
# it loads, too), which a Kenny binary can't (docs/LANGUAGE.md); Discourse requires it. The
# answers are libpsl's registrable domain (the public suffix, private ones included, and one
# label more), in the hostname's own case, as a binary String as FFI returns it.
require "mini_suffix/version"
require "public_suffix"

module MiniSuffix
  # Extracts the shortest private suffix part of the hostname
  def self.domain(hostname)
    return nil if hostname.nil?
    host = hostname.to_s
    dot = host.end_with?(".") ? "." : ""
    core = dot.empty? ? host : host[0...-1]
    labels = core.split(".", -1)
    return nil if labels.empty? || labels.last.empty?
    name = core.downcase
    list = PublicSuffix::List.default
    rule = list.find(name, default: list.default_rule, ignore_private: false)
    # (the suffix's labels: `parts` are those of `x` for `*.x` (and one more) and `!a.x`)
    n = rule.parts.size + (rule.is_a?(PublicSuffix::Rule::Wildcard) ? 1 : 0)
    return nil if labels.size <= n || labels[-(n + 1)].empty?
    (labels[-(n + 1)..].join(".") + dot).b
  end
end
