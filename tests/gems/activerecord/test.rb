require "bundler/setup"
require "active_record"

ActiveRecord::Base.establish_connection(adapter: "sqlite3", database: ":memory:")
ActiveRecord::Migration.verbose = false
ActiveRecord::Schema.define do
  create_table :authors do |t|
    t.string :name, null: false
    t.timestamps
  end
  create_table :posts do |t|
    t.references :author, foreign_key: true
    t.string :title
    t.text :body
    t.integer :views, default: 0
    t.boolean :published, default: false
    t.json :meta
    t.integer :status, default: 0
  end
  create_table :tags do |t| t.string :name end
  create_table :posts_tags, id: false do |t| t.references :post; t.references :tag end
end

class Author < ActiveRecord::Base
  has_many :posts, dependent: :destroy
  has_many :tags, -> { distinct }, through: :posts
  validates :name, presence: true, uniqueness: true
  before_save { self.name = name.strip }
  after_create { (@log ||= []) << :created }
  attr_reader :log
end
class Post < ActiveRecord::Base
  belongs_to :author, counter_cache: false
  has_and_belongs_to_many :tags
  scope :published, -> { where(published: true) }
  scope :popular, ->(min = 10) { where("views >= ?", min) }
  enum :status, { draft: 0, live: 1 }
end
class Tag < ActiveRecord::Base
  has_and_belongs_to_many :posts
end

ann = Author.create!(name: "  Ann ")
bob = Author.create!(name: "Bob")
p ann.name, ann.log, ann.persisted?, Author.count
ruby, rails = Tag.create!(name: "ruby"), Tag.create!(name: "rails")
ann.posts.create!(title: "One", views: 5, published: true, tags: [ruby], meta: { "a" => [1, 2] })
ann.posts.create!(title: "Two", views: 50, published: true, tags: [ruby, rails])
bob.posts.create!(title: "Three", views: 100)
p Post.published.order(:title).pluck(:title), Post.popular.count, Post.popular(60).pluck(:title), Post.published.popular.pluck(:title)
p ann.posts.sum(:views), Post.group(:author_id).count, Post.where(author: bob).first.title, Post.find_by(title: "One").meta
p ann.tags.map(&:name).sort, Tag.find_by(name: "ruby").posts.count, Post.joins(:author).where(authors: { name: "Bob" }).pluck(:title)
p Post.includes(:author).order(views: :desc).map { |x| [x.title, x.author.name] }
p Author.where.not(name: "Ann").pluck(:name), Post.where(views: 10..60).pluck(:title), Post.order(:id).limit(1).offset(1).pluck(:title)
p Author.new(name: "").valid?, Author.new(name: "Bob").tap(&:valid?).errors.full_messages
begin
  Author.transaction do
    Author.create!(name: "Carl")
    raise ActiveRecord::Rollback
  end
end
p Author.exists?(name: "Carl")
begin
  Author.transaction { Author.create!(name: "Dan"); Author.create!(name: nil) }
rescue ActiveRecord::RecordInvalid => e
  p e.message, Author.exists?(name: "Dan")
end
post = Post.first
post.title = "Uno"
p post.changes, post.save, post.saved_change_to_title?, post.reload.title
p Post.update_all(views: 1), Post.sum(:views), Post.maximum(:id), Post.select(:title).distinct.count
ann.destroy; p Post.count, Author.pluck(:name)
p Post.where(title: "Three").to_sql.gsub('"', "'")
p ActiveRecord::Base.connection.select_all("SELECT name FROM tags ORDER BY name").rows
p Post.columns_hash["views"].type, Post.column_names, Author.reflect_on_association(:posts).macro
begin; Post.find(999); rescue ActiveRecord::RecordNotFound => e; p e.message; end
p Post.find_or_create_by(title: "New").persisted?, Post.where(title: "New").exists?
Post.find_each(batch_size: 1) { |x| print x.title, " " }; puts
p Post.first.draft?, Post.live.count, Post.first.tap(&:live!).status, Post.statuses
p Post.arel_table[:views].gt(3).to_sql.gsub('"', "'")
