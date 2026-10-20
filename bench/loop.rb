sum = 0
i = 0
while i < 200_000_000
  sum += i % 7
  i += 1
end
puts sum
