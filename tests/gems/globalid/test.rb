require "bundler/setup"
require "cgi"
require "base64"
require "globalid"

GlobalID.app = "kennyapp"
class Person
  include GlobalID::Identification
  attr_reader :id, :name
  STORE = {}
  def initialize(id, name) = (@id, @name = id, name; STORE[id.to_s] = self)
  def self.find(id) = id.is_a?(Array) ? id.map { STORE.fetch(_1.to_s) } : STORE.fetch(id.to_s)
  def ==(o) = o.is_a?(Person) && o.id == id
end
module Admin; class User < Person; end; end

ann = Person.new(1, "Ann")
bob = Admin::User.new("b-2", "Bob")
gid = ann.to_global_id
p gid.to_s, gid.model_name, gid.model_id, gid.app, gid.uri.class
p GlobalID::Locator.locate(gid).name, GlobalID::Locator.locate("gid://kennyapp/Admin::User/b-2").name
p bob.to_gid_param, GlobalID.parse(bob.to_gid_param).to_s, GlobalID::Locator.locate(bob.to_gid_param).name
p GlobalID::Locator.locate_many([ann.to_gid, bob.to_gid]).map(&:name)
p GlobalID.parse("gid://kennyapp/Person/1?tenant=x").params, GlobalID.new("gid://other/Person/1").app
p GlobalID.parse("not a gid"), GlobalID.find("gid://kennyapp/Person/1").name
p(GlobalID.parse("gid://kennyapp/Person/1") == ann.to_gid)
SignedGlobalID.verifier = GlobalID::Verifier.new("x" * 64)
sgid = ann.to_sgid(for: "login", expires_in: nil)
p sgid.class, SignedGlobalID.parse(sgid.to_s, for: "login").model_id, GlobalID::Locator.locate_signed(sgid.to_s, for: "login").name
p GlobalID::Locator.locate_signed(sgid.to_s, for: "other")
begin
  URI::GID.parse("gid://kennyapp/")
rescue URI::InvalidComponentError, URI::BadURIError => e
  p e.class
end
p URI::GID.build(app: "a", model_name: "M", model_id: "5", params: { k: "v" }).to_s
GlobalID::Locator.use :other do |gid| "located #{gid.model_id}" end
p GlobalID::Locator.locate("gid://other/Anything/7")
