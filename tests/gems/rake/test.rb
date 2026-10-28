require "bundler/setup"
require "rake"
require "tmpdir"
require "fileutils"

Dir.mktmpdir do |dir|
  Dir.chdir(dir) do
    log = []
    app = Rake::Application.new
    Rake.application = app
    app.init("rake", ["--quiet"])
    extend Rake::DSL
    File.write("a.txt", "A")
    File.write("b.txt", "B")

    desc "Build output"
    file "out.txt" => ["a.txt", "b.txt"] do |t|
      log << "build #{t.name} from #{t.prerequisites.join(",")}"
      File.write(t.name, t.prerequisites.map { |f| File.read(f) }.join)
    end

    namespace :db do
      desc "Migrate the db"
      task :migrate, [:version] => :env do |t, args|
        log << "migrate #{args[:version].inspect} #{args.to_h}"
      end
    end
    task :env do
      log << "env"
    end
    task default: ["out.txt", "db:migrate"]
    rule ".o" => ".c" do |t|
      log << "compile #{t.source} -> #{t.name}"
      File.write(t.name, "obj")
    end
    File.write("x.c", "int main;")

    Rake::Task[:default].invoke
    Rake::Task["out.txt"].reenable
    Rake::Task["out.txt"].invoke
    app["db:migrate"].reenable
    app["db:migrate"].invoke("42")
    Rake::Task["x.o"].invoke
    p log
    p File.read("out.txt"), File.exist?("x.o")
    p Rake::Task.tasks.map(&:name)
    p Rake::Task["db:migrate"].comment, Rake::Task["db:migrate"].arg_names, Rake::Task["out.txt"].needed?
    p Rake::FileList["*.txt"].sort.to_a, Rake::FileList["*.c"].ext(".o").to_a
    p "src/foo.c".pathmap("%{src,obj}d/%n.o"), "a/b/c.rb".pathmap("%f %n %x %d")
    begin
      Rake::Task[:nope]
    rescue RuntimeError => e
      puts e.message
    end
    multitask par: [:p1, :p2]
    task(:p1) { log << "p1" }
    task(:p2) { log << "p2" }
    Rake::Task[:par].invoke
    p log.last(2).sort
    app.top_level_tasks.replace(["db:migrate[7]"])
    app["db:migrate"].reenable
    app.top_level
    p log.last
  end
end
