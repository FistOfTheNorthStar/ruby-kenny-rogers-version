require "bundler/setup"
require "faker"

Faker::Config.random = Random.new(42)
Faker::Config.locale = :en
p Faker::Name.name, Faker::Name.first_name, Faker::Internet.email, Faker::Address.city
p Faker::Lorem.sentence, Faker::Lorem.words(number: 3), Faker::Number.number(digits: 6)
p Faker::Number.decimal(l_digits: 2, r_digits: 2), Faker::Number.between(from: 1, to: 10)
p Faker::Alphanumeric.alpha(number: 8), Faker::Color.hex_color, Faker::Company.name
p Faker::Date.between(from: Date.new(2020, 1, 1), to: Date.new(2020, 12, 31))
p Faker::Boolean.boolean, Faker::Internet.uuid.size, Faker::PhoneNumber.cell_phone.class
p Faker::Commerce.price, Faker::Book.title.class, Faker::Lorem.paragraph(sentence_count: 2).count(".")
p Faker::Config.random.rand(1000)
Faker::Config.locale = :de
p Faker::Address.city, Faker::Name.last_name
Faker::Config.locale = :ja
p Faker::Name.name.encoding
Faker::Config.locale = :en
u = Faker::UniqueGenerator.new(Faker::Number, 10)
p 5.times.map { Faker::Number.unique.between(from: 1, to: 5) }.sort
Faker::UniqueGenerator.clear
p Faker::Base.bothify("??-##"), Faker::Base.regexify(/[A-C]{3}\d{2}/)
p Faker::Internet.password(min_length: 10, max_length: 12, mix_case: true, special_characters: false).size.between?(10, 12)
