# DidYouMean: loaded on first use (CRuby loads it at startup), suggestions in detailed_message
p defined?(DidYouMean), defined?(DidYouMean::Correctable), defined?(Set)
def hello = 1
p [:frist, :hello, :fo].zip([(([1].frist) rescue $!), ((helo) rescue $!), (({foo: 1}.fetch(:fo)) rescue $!)]).map { |n, e| e.detailed_message(highlight: false).include?("Did you mean?") }
p DidYouMean::SpellChecker.new(dictionary: %w[create update destroy]).correct("updat")
e = (Zork rescue $!)
p e.name, e.receiver, NameError.new("x").respond_to?(:local_variables)
p (ObjectSpace rescue $!).class
