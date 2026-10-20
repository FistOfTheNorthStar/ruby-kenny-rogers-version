class Engine < Railtie
  class Configuration < Railtie::Configuration
    def initialize(root)
      @root = root
    end
    attr_reader :root
  end
end
