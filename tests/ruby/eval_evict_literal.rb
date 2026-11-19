# frozen_string_literal: true

# A frozen literal is one String for the whole program (compiled code and eval'd code
# share it). Evicting eval units from the eval cache (past 2000) and freeing them must not
# let it be collected: webmock's `HTTP::VERSION < "1.0.0"` found an Array there.
def lit = "kenny-shared-literal"
lit_id = lit.object_id
3.times do |round|
  2200.times do |i|
    eval("# frozen_string_literal: true\n[\"kenny-shared-literal\", #{round * 10_000 + i}]")
  end
  GC.start
  junk = Array.new(20_000) { |i| [i, i.to_s] }
  junk.size
end
GC.start
p lit.class, lit, lit.object_id == lit_id, lit.frozen?, eval("# frozen_string_literal: true\n'kenny-shared-literal'").equal?(lit)
p (-"kenny-shared-literal".dup).equal?(lit), "1.0.0" < "6.0.0"
