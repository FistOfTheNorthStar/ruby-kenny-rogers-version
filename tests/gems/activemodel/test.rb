require "bundler/setup"
require "active_model"

class Signup
  include ActiveModel::Model
  include ActiveModel::Attributes
  include ActiveModel::Serializers::JSON
  include ActiveModel::Dirty

  attribute :email, :string
  attribute :age, :integer, default: 18
  attribute :born_on, :date
  attribute :admin, :boolean, default: false
  attribute :score, :decimal

  validates :email, presence: true, format: { with: /@/ }
  validates :age, numericality: { greater_than_or_equal_to: 13 }
  validate :no_bad_domain
  validates_length_of :email, maximum: 20

  def no_bad_domain
    errors.add(:email, :invalid, message: "has a bad domain") if email.to_s.end_with?("@bad.com")
  end
end

s = Signup.new(email: "kenny@example.com", age: "42", born_on: "1990-05-06", admin: "1", score: "1.50")
p s.valid?, s.age, s.born_on, s.admin, s.score.to_s, s.attributes.keys
p s.as_json, s.to_json
bad = Signup.new(email: "x", age: 5)
p bad.valid?, bad.errors.full_messages, bad.errors[:age], bad.errors.details[:email]
p Signup.new(email: "aaaaaaaaaaaaaaaaaaaaaaa@bad.com").tap(&:valid?).errors.full_messages
p Signup.model_name.human, Signup.model_name.param_key, Signup.model_name.route_key, Signup.human_attribute_name(:born_on)
p s.persisted?, s.to_key, s.to_param, s.model_name.singular

s.changes_applied
s.email = "new@example.com"
p s.changed?, s.changed, s.email_was, s.changes, s.email_changed?(from: "kenny@example.com")
s.restore_attributes; p s.email, s.changed?

class Pw
  include ActiveModel::API
  include ActiveModel::Validations::Callbacks
  attr_accessor :password, :name, :log
  before_validation { (self.log ||= []) << :before }
  validates :name, inclusion: { in: %w[a b] }, allow_nil: true
  validates :password, confirmation: true
end
pw = Pw.new(name: "c", password: "secret", password_confirmation: "nope")
p pw.valid?, pw.errors.full_messages, pw.log
class Thing
  extend ActiveModel::Callbacks
  include ActiveModel::Conversion
  define_model_callbacks :create
  before_create { puts "before create" }
  after_create { puts "after create" }
  def create = run_callbacks(:create) { puts "creating" }
end
Thing.new.create
p ActiveModel::Type::Integer.new.cast("12abc"), ActiveModel::Type::Boolean.new.cast("false"), ActiveModel::Type::Date.new.cast("2020-02-30")
p ActiveModel::Name.new(Signup, nil, "Account::Signup").plural
e = ActiveModel::Errors.new(s); e.add(:base, "whole thing"); p e.full_messages, e.count, e.to_hash
