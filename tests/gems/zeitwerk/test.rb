require "bundler/setup"
require "zeitwerk"
require "tmpdir"
require "fileutils"

Dir.mktmpdir do |dir|
  files = {
    "app/models/user.rb" => "class User; def self.kind = :model; end",
    "app/models/admin/role.rb" => "module Admin; class Role; NAME = 'role'; end; end",
    "app/models/admin.rb" => "module Admin; def self.ok? = true; end",
    "app/services/billing/invoice_maker.rb" => "module Billing; class InvoiceMaker; def call = User.kind; end; end",
    "app/services/html_parser.rb" => "class HTMLParser; end",
    "lib/my_gem/version.rb" => "module MyGem; class Version; STRING = %q(1.0); end; end",
  }
  files.each { |path, src| FileUtils.mkdir_p(File.join(dir, File.dirname(path))); File.write(File.join(dir, path), src) }
  loader = Zeitwerk::Loader.new
  loader.push_dir(File.join(dir, "app/models"))
  loader.push_dir(File.join(dir, "app/services"))
  loader.push_dir(File.join(dir, "lib"))
  loader.inflector.inflect("html_parser" => "HTMLParser")
  loader.enable_reloading
  loaded = []
  loader.on_load { |cpath, _value, _abspath| loaded << cpath }
  loader.setup
  p Object.autoload?(:User).nil?, defined?(User), User.kind, Admin.ok?, Admin::Role::NAME
  p Billing::InvoiceMaker.new.call, HTMLParser.name, MyGem::Version::STRING
  p loaded
  p loader.cpath_expected_at(File.join(dir, "app/services/billing/invoice_maker.rb"))
  loader.reload
  p Object.const_defined?(:User, false), User.kind
  loader.eager_load
  p loaded.sort.uniq
  p Zeitwerk::Inflector.new.camelize("users_controller", nil), loader.tag.class
  loader.unload
  p defined?(MyGem)
end
