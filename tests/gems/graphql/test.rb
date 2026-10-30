require "bundler/setup"
require "graphql"

AUTHORS = { 1 => { id: 1, name: "Le Guin" }, 2 => { id: 2, name: "Pratchett" } }
BOOKS = [{ id: 10, title: "Earthsea", author_id: 1, year: 1968 }, { id: 11, title: "Mort", author_id: 2, year: 1987 },
         { id: 12, title: "The Lathe of Heaven", author_id: 1, year: 1971 }]

class GenreEnum < GraphQL::Schema::Enum
  value "FANTASY"
  value "SF", value: :science_fiction
end
class AuthorType < GraphQL::Schema::Object
  field :id, ID, null: false
  field :name, String, null: false
  field :books, ["BookType"], null: false
  def books = BOOKS.select { |b| b[:author_id] == object[:id] }
end
class BookType < GraphQL::Schema::Object
  field :id, ID, null: false
  field :title, String, null: false
  field :year, Integer, null: true
  field :author, AuthorType, null: false
  field :shout, String, null: false do
    argument :times, Integer, required: false, default_value: 1
  end
  def author = AUTHORS[object[:author_id]]
  def shout(times:) = object[:title].upcase * times
end
class BookInput < GraphQL::Schema::InputObject
  argument :title, String
  argument :year, Integer, required: false
end
class AddBook < GraphQL::Schema::Mutation
  argument :input, BookInput
  argument :author_id, ID
  field :book, BookType
  field :errors, [String], null: false
  def resolve(input:, author_id:)
    return { book: nil, errors: ["no such author"] } unless AUTHORS[author_id.to_i]
    b = { id: 13, title: input.title, year: input.year, author_id: author_id.to_i }
    { book: b, errors: [] }
  end
end
class QueryType < GraphQL::Schema::Object
  field :books, [BookType], null: false do
    argument :since, Integer, required: false
  end
  field :author, AuthorType do
    argument :id, ID
  end
  field :genre, GenreEnum, null: false
  def books(since: 0) = BOOKS.select { |b| b[:year] >= since }
  def author(id:) = AUTHORS[id.to_i]
  def genre = :science_fiction
end
class MutationType < GraphQL::Schema::Object
  field :add_book, mutation: AddBook
end
class Schema < GraphQL::Schema
  query QueryType
  mutation MutationType
end

def run(q, **opts) = p(Schema.execute(q, **opts).to_h)
run("{ books { title year author { name } } }")
run("query($s: Int) { books(since: $s) { title shout(times: 2) } }", variables: { "s" => 1970 })
run('{ author(id: "1") { name books { title } } genre }')
run('mutation { addBook(authorId: "2", input: { title: "Guards! Guards!", year: 1989 }) { book { id title author { name } } errors } }')
run('mutation { addBook(authorId: "9", input: { title: "X" }) { book { id } errors } }')
run("{ books { nope } }")
run("{ books(since: \"x\") { title } }")
run("{ __type(name: \"Book\") { name fields { name type { kind name ofType { name } } } } }")
run("query Named { ...F } fragment F on Query { books { id } }")
puts Schema.to_definition
p GraphQL.parse("{ a(b: 1) { c } }").definitions.first.selections.first.name
