# blog8

A generated Rails 8.1 app compiled by Kenny: `rails _8.1.4_ new blog8 --skip-git
--skip-docker --skip-kamal --skip-thruster --skip-ci --skip-test --skip-system-test`, then
`rails g scaffold Post title:string body:text published:boolean`,
`rails g scaffold Comment post:references author:string body:text` and `root "posts#index"`.
The only other change: config/database.yml names the production databases, which the
generator leaves out.

With Ruby 3.3+ and the app's gems installed (`bundle install`), from the repository root:

```sh
(cd examples/blog8 && RAILS_ENV=production SECRET_KEY_BASE=x bin/rails db:prepare assets:precompile)
./target/release/kenny build examples/blog8/config.ru -o blog8
(cd examples/blog8 && RAILS_ENV=production SECRET_KEY_BASE=$(openssl rand -hex 64) ../../blog8)
ruby examples/blog8/compare.rb blog8   # the same session against Puma and the binary
```
