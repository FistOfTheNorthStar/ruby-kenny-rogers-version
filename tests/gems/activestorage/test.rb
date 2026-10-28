require "bundler/setup"
require "active_record/railtie"
require "active_storage/engine"
require "tmpdir"
require "rack/mock"

STORAGE = Dir.mktmpdir
class StorageApp < Rails::Application
  config.root = __dir__
  config.eager_load = false
  config.logger = Logger.new(nil)
  config.secret_key_base = "s" * 64
  config.active_storage.service = :local
  config.active_storage.service_configurations = { local: { service: "Disk", root: STORAGE } }
  config.active_storage.variant_processor = :disabled rescue nil
  config.hosts.clear
end
ENV["DATABASE_URL"] = "sqlite3::memory:"
StorageApp.initialize!
ActiveRecord::Migration.verbose = false
ActiveRecord::Schema.define do
  create_table :active_storage_blobs do |t|
    t.string :key, null: false
    t.string :filename, null: false
    t.string :content_type
    t.text :metadata
    t.string :service_name, null: false
    t.bigint :byte_size, null: false
    t.string :checksum
    t.datetime :created_at, null: false
    t.index [:key], unique: true
  end
  create_table :active_storage_attachments do |t|
    t.string :name, null: false
    t.references :record, null: false, polymorphic: true, index: false
    t.references :blob, null: false
    t.datetime :created_at, null: false
  end
  create_table :active_storage_variant_records do |t|
    t.belongs_to :blob, null: false, index: false
    t.string :variation_digest, null: false
  end
  create_table :users do |t| t.string :name end
end

class User < ActiveRecord::Base
  has_one_attached :avatar
  has_many_attached :documents
end

u = User.create!(name: "ann")
p u.avatar.attached?
u.avatar.attach(io: StringIO.new("hello avatar"), filename: "a.txt", content_type: "text/plain")
p u.avatar.attached?, u.avatar.filename.to_s, u.avatar.content_type, u.avatar.byte_size, u.avatar.download, u.avatar.checksum
u.documents.attach([{ io: StringIO.new("one"), filename: "1.csv" }, { io: StringIO.new("two!"), filename: "2.json" }])
p u.documents.map { [_1.filename.to_s, _1.byte_size, _1.content_type] }, ActiveStorage::Blob.count, ActiveStorage::Attachment.count
blob = ActiveStorage::Blob.create_and_upload!(io: StringIO.new("<b>x</b>"), filename: "x.html")
p blob.content_type, blob.service_name, blob.filename.extension, blob.filename.base, ActiveStorage::Filename.new("../evil/na me.txt").sanitized
p blob.key.size, File.exist?(ActiveStorage::Blob.service.path_for(blob.key)), blob.open { |f| f.read }
p u.reload.avatar.blob.metadata.slice("identified"), User.with_attached_documents.first.documents.size
signed = blob.signed_id; p ActiveStorage::Blob.find_signed(signed) == blob
u.avatar.purge
p u.reload.avatar.attached?, ActiveStorage::Blob.count, Rails.application.routes.url_helpers.respond_to?(:rails_blob_path)
path = Rails.application.routes.url_helpers.rails_blob_path(blob, only_path: true)
p path.sub(/blobs\/redirect\/[^\/]+/, "blobs/redirect/SIGNED")
res = Rack::MockRequest.new(Rails.application).get(path)
p res.status, res.headers["location"].to_s.sub(%r{/disk/[^/]+/}, "/disk/TOKEN/")
