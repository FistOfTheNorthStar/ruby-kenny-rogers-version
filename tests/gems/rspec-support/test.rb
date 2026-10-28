require "bundler/setup"
require "rspec/support"
RSpec::Support.require_rspec_support "object_formatter"
RSpec::Support.require_rspec_support "fuzzy_matcher"
RSpec::Support.require_rspec_support "method_signature_verifier"
RSpec::Support.require_rspec_support "source"

fmt = RSpec::Support::ObjectFormatter.new(40)
p fmt.format("x" * 100)
p RSpec::Support::ObjectFormatter.format(Time.utc(2024, 1, 2, 3, 4, 5))
p RSpec::Support::ObjectFormatter.format([1.5, :a, nil, { b: "c" }])
p RSpec::Support::ObjectFormatter.format(Class.new)[0, 8]

FM = RSpec::Support::FuzzyMatcher
p FM.values_match?(Integer, 3)
p FM.values_match?(/ab/, "cab")
p FM.values_match?([1, String], [1, "x"])
p FM.values_match?({ a: (1..5) }, { a: 3 })
p FM.values_match?({ a: 1 }, { a: 1, b: 2 })

class Signed
  def m(a, b = 1, *rest, c:, d: 2, **opts, &blk); end
  def n(x); end
end
sig = RSpec::Support::MethodSignature.new(Signed.instance_method(:m))
p sig.description
p sig.min_non_kw_args, sig.max_non_kw_args
p sig.required_kw_args
v = RSpec::Support::MethodSignatureVerifier.new(sig, [1, { c: 3 }])
p v.valid?
v = RSpec::Support::StrictSignatureVerifier.new(RSpec::Support::MethodSignature.new(Signed.instance_method(:n)), [1, 2])
p v.valid?
p v.error_message

p RSpec::Support.method_handle_for(Signed.new, :n).arity
p RSpec::Support::RubyFeatures.required_kw_args_supported?
p RSpec::Support.class_of(3)
p RSpec::Support::Ruby.mri?
p RSpec::Support::ComparableVersion.new("3.13.6") > RSpec::Support::ComparableVersion.new("3.9")
RSpec::Support.with_failure_notifier(->(failure, _opts) { puts "notified: #{failure.message}" }) do
  RSpec::Support.notify_failure(RuntimeError.new("soft failure"))
end
begin
  RSpec::Support.notify_failure(RuntimeError.new("hard failure"))
rescue => e
  p [e.class, e.message]
end
p RSpec::Support.thread_local_data.class
