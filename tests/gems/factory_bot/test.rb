require "bundler/setup"
require "factory_bot"

class User
  attr_accessor :name, :email, :admin, :age, :posts, :created
  def initialize(**kw) = kw.each { |k, v| public_send("#{k}=", v) }
  def save! = (@saved = true)
  def persisted? = !!@saved
end
class Post
  attr_accessor :title, :author, :body
  def save! = (@saved = true)
end

FactoryBot.define do
  sequence(:email) { |n| "user#{n}@example.com" }
  factory :user do
    name { "Ann" }
    email { generate(:email) }
    admin { false }
    age { 30 }
    initialize_with { new(**attributes) }
    trait :admin do
      admin { true }
      name { "Root" }
    end
    trait(:old) { age { 99 } }
    factory :admin_user, traits: [:admin]
    transient do
      post_count { 0 }
    end
    after(:build) { |u, ev| u.posts = Array.new(ev.post_count) { |i| "post #{i}" } }
  end
  factory :post do
    sequence(:title) { |n| "Title #{n}" }
    body { "#{title} body" }
    association :author, factory: :user, strategy: :build
  end
end

u = FactoryBot.build(:user)
p [u.name, u.email, u.admin, u.age, u.posts, u.persisted?]
a = FactoryBot.build(:user, :admin, :old, name: "Override")
p [a.name, a.admin, a.age, a.email]
p FactoryBot.build(:admin_user).name
p FactoryBot.build(:user, post_count: 2).posts
p FactoryBot.attributes_for(:user, :old).sort.to_h.except(:email)
list = FactoryBot.build_list(:user, 3)
p list.map(&:email)
c = FactoryBot.create(:user)
p c.persisted?
po = FactoryBot.build(:post)
p [po.title, po.body, po.author.class, po.author.name]
pairs = FactoryBot.build_pair(:post)
p pairs.map(&:title)
begin
  FactoryBot.build(:nope)
rescue KeyError => e
  p e.class
end
p FactoryBot.factories.map(&:name).sort
