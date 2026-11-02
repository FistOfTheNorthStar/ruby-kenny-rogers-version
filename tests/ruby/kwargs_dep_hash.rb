# kenny-run-flags: -I tests/ruby/kwargs_dep_hash
# A Hash passed positionally to a library method with keywords: taken as the keywords only when
# it holds just keywords the method takes (Kenny's trailing-Hash rule for dependencies),
# otherwise it stays positional, as in Ruby 3 (Discourse's DiscourseMcp object_schema)
require "dep_schema"

opts = { flag: true }
p DepSchema.object_schema({ filter: { type: "string" }, view: { type: "string" } })
p DepSchema.object_schema({ filter: 1 }, required: %w[filter])
p DepSchema.object_schema
p DepSchema.opts(1, { other: 2 })
p DepSchema.opts(1, { other: 2 }, flag: true)
p DepSchema.opts(1, **opts)
p DepSchema.rest(1, { a: 1 })
p DepSchema.rest(1, mode: :x)
p DepSchema.legacy(1, flag: true)
p DepSchema.block_call { |h = {}, name: nil| [h, name] }
p DepSchema.block_kw { |k: 0| k }
