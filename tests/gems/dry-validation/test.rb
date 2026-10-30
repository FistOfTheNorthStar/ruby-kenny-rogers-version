require "bundler/setup"
require "dry-validation"

class NewUserContract < Dry::Validation::Contract
  params do
    required(:email).filled(:string)
    required(:age).value(:integer)
    optional(:name).maybe(:string)
    required(:address).hash do
      required(:city).filled(:string)
      optional(:zip).filled(:string, size?: 5)
    end
    optional(:tags).array(:symbol)
    required(:role).value(included_in?: %w[user admin])
  end

  rule(:email) do
    key.failure("has invalid format") unless /\A[\w.+-]+@[\w-]+\.\w+\z/.match?(value)
  end
  rule(:age) do
    key.failure("must be greater than 18") if value <= 18
  end
  rule(:role, :age) do
    key(:role).failure("admins must be 21+") if values[:role] == "admin" && values[:age] < 21
  end
  rule(address: :zip) do
    key.failure("must start with 9") if key? && !value.start_with?("9")
  end
end

c = NewUserContract.new
ok = c.call("email" => "jane@doe.org", "age" => "30", "address" => { "city" => "Austin", "zip" => "90210" }, "role" => "user", "tags" => ["a"])
p ok.success?, ok.to_h, ok[:age], ok.errors.to_h
bad = c.call("email" => "nope", "age" => "17", "address" => { "city" => "", "zip" => "123" }, "role" => "boss")
p bad.failure?, bad.errors.to_h
p bad.errors.messages.map { |m| [m.path, m.text] }
p bad.errors(full: true).to_h
p c.call("email" => "a@b.cc", "age" => "19", "address" => { "city" => "X" }, "role" => "admin").errors.to_h
p c.call({}).errors.to_h
p c.call("email" => "a@b.cc", "age" => "abc", "address" => "x", "role" => "user").errors.to_h
p c.call("email" => "a@b.cc", "age" => "40", "address" => { "city" => "X", "zip" => "12345" }, "role" => "user").errors.to_h

Schema = Dry::Schema.Params do
  required(:items).array(:hash) do
    required(:sku).filled(:string)
    required(:qty).filled(:integer, gt?: 0)
  end
  optional(:note).maybe(:str?)
end
r = Schema.call("items" => [{ "sku" => "A", "qty" => "2" }, { "sku" => "", "qty" => "0" }])
p r.success?, r.to_h, r.errors.to_h
j = Dry::Schema.JSON { required(:flag).filled(:bool); required(:n).value(:float) }
p j.call("flag" => true, "n" => 1).to_h, j.call("flag" => "true", "n" => "x").errors.to_h

class WithOpts < Dry::Validation::Contract
  option :min
  json { required(:count).filled(:integer) }
  rule(:count) { key.failure("must be >= #{min}") if value < min }
end
p WithOpts.new(min: 5).call("count" => 3).errors.to_h, WithOpts.new(min: 1).call("count" => 3).success?

class Base < Dry::Validation::Contract
  schema { required(:id).filled(:integer) }
  rule(:id) { base.failure("id must be even") if value.odd? }
end
res = Base.new.call(id: 3)
p res.errors.to_h, res.errors.filter(:base?).map(&:text), res.error?(:id)
class Macro < Dry::Validation::Contract
  register_macro(:positive) { key.failure("must be positive") unless value.positive? }
  params { required(:n).filled(:integer) }
  rule(:n).validate(:positive)
end
p Macro.new.call(n: "-3").errors.to_h, Macro.new.("n" => "4").to_h
begin
  NewUserContract.new.call(nil)
rescue => e
  puts "#{e.class}"
end
