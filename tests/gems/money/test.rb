require "bundler/setup"
require "money"

Money.locale_backend = :currency
Money.rounding_mode = BigDecimal::ROUND_HALF_EVEN
Money.default_currency = "USD"
Money.default_bank = Money::Bank::VariableExchange.new(Money::RatesStore::Memory.new)
Money.default_bank.add_rate("USD", "EUR", 0.9)
Money.default_bank.add_rate("EUR", "USD", 1.1111)
a = Money.new(1050, "USD")
b = Money.from_amount(3.25, "USD")
p a.cents, a.amount, a.currency.iso_code, a.format, b.format
p (a + b).format, (a - b).to_s, (a * 3).format, (a / 4).format, -a
p a > b, a == Money.new(1050), a.zero?, Money.new(0).zero?, a.positive?
p a.allocate([1, 1, 1]).map(&:cents), a.split(3).map(&:cents)
p Money.new(100, "JPY").format, Money.new(123456789, "EUR").format
p Money.new(123456, "EUR").format(symbol: false), Money.new(123456, "GBP").format(with_currency: true)
p Money.new(100_000_00, "USD").format(no_cents: true), Money.new(5, "USD").format(thousands_separator: ".", decimal_mark: ",")
p Money.new(1000, "BHD").format, Money.new(1000, "BHD").currency.exponent
c = Money::Currency.new("CHF")
p c.name, c.symbol, c.subunit, c.subunit_to_unit, c.iso_numeric, c.to_s
p Money::Currency.find_by_iso_numeric(978).iso_code
p Money.new(1, "USD").currency == Money::Currency.new(:usd)

p a.exchange_to("EUR").format, Money.new(900, "EUR").exchange_to(:usd).cents
p Money.default_bank.export_rates(:json)
begin
  Money.new(100, "USD").exchange_to("GBP")
rescue Money::Bank::UnknownRate => e
  puts "UnknownRate: #{e.message}"
end
begin
  Money::Currency.new("XYZ")
rescue Money::Currency::UnknownCurrency => e
  puts "UnknownCurrency: #{e.message}"
end
begin
  Money.new(1, "USD") + Money.new(1, "GBP")
rescue => e
  puts "#{e.class}: #{e.message}"
end
p Money.new(1999, "USD").round_to_nearest_cash_value rescue p $!.class
p Money.from_cents(12345, "USD").to_d, Money.new(1.5, "USD").cents
p [Money.new(3), Money.new(1), Money.new(2)].sort.map(&:cents), [Money.new(3), Money.new(1)].sum(Money.new(0)).format
p Money.new(1000, "USD").hash == Money.new(1000, "USD").hash
Money::Currency.register(iso_code: "KNY", name: "Kenny Dollar", symbol: "K$", subunit_to_unit: 100, priority: 100, symbol_first: true, thousands_separator: ",", decimal_mark: ".")
p Money.new(250099, "KNY").format, Money::Currency.new("KNY").name
p Money.new(1234567, "USD").format(format: "%n %u"), Money.new(-1000, "USD").format(sign_before_symbol: true), Money.new(1050).to_nearest_cash_value.fractional
