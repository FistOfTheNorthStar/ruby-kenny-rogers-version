require "bundler/setup"
require "sequel"

DB = Sequel.sqlite
DB.create_table(:authors) do
  primary_key :id
  String :name, null: false, unique: true
  Integer :born
end
DB.create_table(:books) do
  primary_key :id
  foreign_key :author_id, :authors
  String :title
  Float :price
  TrueClass :in_print, default: true
end

Sequel::Model.plugin :validation_helpers
class Author < Sequel::Model
  one_to_many :books
  def validate
    super
    validates_presence :name
    validates_min_length 2, :name
  end
end
class Book < Sequel::Model
  many_to_one :author
  dataset_module do
    def cheap = where { price < 10 }
    def by_title = order(:title)
  end
end

le = Author.create(name: "Le Guin", born: 1929)
pk = Author.create(name: "Pratchett", born: 1948)
le.add_book(title: "The Dispossessed", price: 12.5)
le.add_book(title: "A Wizard of Earthsea", price: 8.0)
pk.add_book(title: "Mort", price: 7.25, in_print: false)
p Author.count, Book.count
p Book.cheap.by_title.map(:title)
p le.books.map(&:title).sort, Book.first(title: "Mort").author.name
p Author.where(Sequel.like(:name, "P%")).select_map(:name)
p DB[:books].group_and_count(:author_id).order(:author_id).all
p DB[:books].sum(:price), DB[:books].avg(:price).round(3), DB[:books].max(:title)
p Book.where(in_print: false).count
a = Author.new(name: "X")
p a.valid?, a.errors.full_messages
begin
  Author.create(name: "Le Guin")
rescue Sequel::UniqueConstraintViolation => e
  p e.class
end
DB.transaction do
  Author.create(name: "Rollback")
  raise Sequel::Rollback
end
p Author.where(name: "Rollback").count
p DB[:books].join(:authors, id: :author_id).order(Sequel[:books][:id]).select(Sequel[:books][:title], Sequel[:authors][:name]).map(:name)
p DB[:authors].where(born: 1900..1940).sql
p Book.eager(:author).all.map { |b| b.author.name }
ds = DB[:books].where(Sequel.lit("price > ?", 7.5)).exclude(title: nil).reverse(:price)
p ds.map(:price)
le.update(born: 1930)
p Author[le.id].born, le.to_hash.keys.sort
p DB.tables.sort, DB.schema(:books).map { |c, i| [c, i[:type]] }
