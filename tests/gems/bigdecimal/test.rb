require "bundler/setup"
require "bigdecimal"
require "bigdecimal/util"

a = BigDecimal("1.1")
b = BigDecimal("2.2")
p a + b
puts (a + b).to_s
puts (a + b).to_s("F")
p (a + b) == BigDecimal("3.3")
p 0.1 + 0.2 == 0.3, BigDecimal("0.1") + BigDecimal("0.2") == BigDecimal("0.3")
x = BigDecimal("123.456789")
p x.round(2), x.round(2).to_s("F"), x.floor, x.ceil, x.truncate(3).to_s
p x.round(2, :banker).to_s, BigDecimal("2.5").round(0, BigDecimal::ROUND_HALF_EVEN).to_s, BigDecimal("2.5").round.class
p BigDecimal("1").div(BigDecimal("3"), 20).to_s
p (BigDecimal("1") / 3).to_s
p BigDecimal("10").sqrt(10).round(10).to_s
p BigDecimal("1234567.891").to_s("3F")
p BigDecimal("-0.00012").to_s, BigDecimal("-0.00012").to_s("E"), BigDecimal("1e20").to_s("F")
p BigDecimal("3.14159").to_f, BigDecimal("42").to_i, BigDecimal("42.9").to_i
p BigDecimal("1.50").precision, BigDecimal("1.50").scale, BigDecimal("1.5").exponent
p BigDecimal("NaN").nan?, BigDecimal("Infinity").infinite?, BigDecimal("-0").zero?
p "12.34".to_d, 5.to_d, 1.25.to_d, "abc".to_d
p BigDecimal("2") ** 100
p BigDecimal("1.23456789").mult(BigDecimal("2"), 4).to_s
p BigDecimal("100").sub(BigDecimal("0.001"), 3).to_s
p (BigDecimal("7") % 3).to_s, BigDecimal("7").divmod(BigDecimal("2")).map(&:to_s)
p BigDecimal("1.5") <=> 1.5, BigDecimal("1.5") > 1, [BigDecimal("3"), BigDecimal("1"), BigDecimal("2")].sort.map(&:to_i)
p BigDecimal("0.1").hash == BigDecimal("0.10").hash
p BigDecimal("12.5").to_r, BigDecimal("12.5").to_d.class, BigDecimal("12.5").frac.to_s, BigDecimal("-12.5").abs.to_s
p BigDecimal("1.23").sign, BigDecimal("-1.23").sign, BigDecimal("0").sign
begin
  BigDecimal("not a number")
rescue ArgumentError => e
  puts e.message
end
BigDecimal.save_rounding_mode do
  BigDecimal.mode(BigDecimal::ROUND_MODE, :down)
  p BigDecimal("2.99").round(1).to_s
end
p BigDecimal("2.99").round(1).to_s
p [BigDecimal("1.1"), BigDecimal("2.2")].sum.to_s
p BigDecimal("1e-10").to_s("F"), BigDecimal("123e-2").inspect
