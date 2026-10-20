a = Rational(1, 3)
b = 2/4r
p a, b, a + b, a - b, a * b, a / b, a + 1, 1 + a, a * 2.0
p a < b, a == Rational(2, 6), 3r, 1.5r, Rational("0.75"), Rational("3/4")
p a.to_f.round(5), (7/3r).floor, (7/3r).ceil, (7/3r).round, (-7/2r).round, (7/3r).round(2)
p 0.5.to_r, 0.1.rationalize, 3.to_r, 1.quo(3), (1/3r).numerator, (1/3r).denominator
p [Rational(1, 2), Rational(1, 3), Rational(1, 6)].sum, Rational(5, 10).inspect, Rational(5, 10).to_s
