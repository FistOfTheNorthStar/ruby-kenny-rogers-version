require "bundler/setup"
require "pundit"

User = Struct.new(:name, :admin) do
  def admin? = admin
end
Post = Struct.new(:title, :author, :published) do
  def self.policy_class = PostPolicy
  def self.all = $posts
end
class Comment
  attr_reader :post
  def initialize(post) = @post = post
end

class ApplicationPolicy
  attr_reader :user, :record
  def initialize(user, record)
    @user = user
    @record = record
  end
  def index? = true
  def show? = false
  def update? = false
  def destroy? = false
end

class PostPolicy < ApplicationPolicy
  def show? = record.published || owner?
  def update? = owner? || user.admin?
  def destroy? = user.admin?
  def permitted_attributes = user.admin? ? [:title, :author, :published] : [:title]
  def owner? = record.author == user.name

  class Scope
    def initialize(user, scope)
      @user = user
      @scope = scope
    end
    def resolve = @user.admin? ? @scope.all : @scope.all.select(&:published)
  end
end

class CommentPolicy < ApplicationPolicy
  def show? = Pundit.policy!(user, record.post).show?
end

alice = User.new("alice", false)
admin = User.new("root", true)
$posts = posts = [Post.new("a", "alice", false), Post.new("b", "bob", true), Post.new("c", "bob", false)]

p Pundit.policy(alice, posts[0]).class, Pundit.policy!(alice, posts[0]).update?
p Pundit.policy_scope(alice, Post).map(&:title), Pundit.policy_scope!(admin, Post).map(&:title)
p Pundit.authorize(alice, posts[0], :update?).title
begin
  Pundit.authorize(alice, posts[1], :update?)
rescue Pundit::NotAuthorizedError => e
  puts "NotAuthorizedError: #{e.message}"
  p e.query, e.record.title, e.policy.class
end
p Pundit.policy(alice, Comment.new(posts[1])).show?, Pundit.policy(alice, Comment.new(posts[2])).show?
p Pundit.policy(alice, Object.new)
begin
  Pundit.policy!(alice, Object.new)
rescue Pundit::NotDefinedError => e
  puts "NotDefinedError: #{e.message.sub(/0x\h+/, 'X')}"
end
p Pundit::PolicyFinder.new(posts[0]).policy, Pundit::PolicyFinder.new(Comment).param_key
p Pundit::PolicyFinder.new([:admin, posts[0]]).policy rescue p $!.class

class Controller
  include Pundit::Authorization
  attr_accessor :current_user, :action_name
  def initialize(user) = (@current_user = user; @action_name = "update")
  public :authorize, :policy_scope, :policy, :verify_authorized, :verify_policy_scoped, :permitted_attributes, :pundit_user
  def params = nil
end
c = Controller.new(alice)
p c.pundit_user.name, c.authorize(posts[0]).title, c.policy(posts[0]).show?
c.verify_authorized
begin
  c.verify_policy_scoped
rescue Pundit::PolicyScopingNotPerformedError => e
  puts "#{e.class}: #{e.message}"
end
p c.policy_scope(Post).size
begin
  Controller.new(alice).verify_authorized
rescue Pundit::AuthorizationNotPerformedError => e
  puts "#{e.class}: #{e.message}"
end
begin
  c.authorize(posts[1], :destroy?)
rescue Pundit::NotAuthorizedError => e
  puts e.message
end
p Pundit.policy(admin, posts[1]).permitted_attributes, Pundit.policy(alice, posts[1]).permitted_attributes
