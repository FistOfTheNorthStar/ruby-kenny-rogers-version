require "bundler/setup"
require "netrc"
require "tmpdir"

Dir.mktmpdir do |dir|
  path = File.join(dir, ".netrc")
  File.write(path, <<~NETRC)
    machine example.com
      login alice
      password s3cret
    machine api.example.org login bob password hunter2
    # a comment
    default login anon password guest
  NETRC
  File.chmod(0600, path)
  n = Netrc.read(path)
  p n["example.com"].to_a, n["api.example.org"].login, n["api.example.org"].password
  p n["nope.example"]
  n["new.example"] = "carol", "pw"
  n["example.com"] = "alice2", "newpass"
  n.delete("api.example.org")
  n.save
  puts File.read(path)
  m = Netrc.read(path)
  p m["new.example"].to_a, m["example.com"].to_a
  e = m["example.com"]
  p e.class, e.login, e.password
  File.chmod(0644, path)
  begin
    Netrc.read(path)
  rescue Netrc::Error => err
    p err.message.sub(path, "PATH")
  end
  p Netrc.read(File.join(dir, "missing")).instance_variable_get(:@data)
end
p Netrc::VERSION
