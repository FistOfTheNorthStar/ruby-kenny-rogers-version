# The test suite on Linux, in a container: Debian with CRuby, Rust, and the gems the tests use.
#
#   docker build -t kenny-linux tools/linux                      (--build-arg RUBY=4.0.9 for 4.0)
#   docker run --rm -v "$PWD":/src -v kenny-target:/src/target kenny-linux tests/run.sh
#
# (the volume keeps the Linux build apart from the host's target/; with Ruby 4.0 add
# -e KENNY_RUBY=/usr/local/bin/ruby, since .ruby-version names 3.4)
ARG RUBY=3.4.12
FROM ruby:${RUBY}-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends clang libclang-dev cmake pkg-config && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH=/root/.cargo/bin:$PATH
WORKDIR /src
RUN gem install --no-document -i "$(ruby -e "print Gem.default_dir")" sqlite3:1.7.3 sassc:2.4.0 execjs:2.10.0 rack:3.2.8
CMD ["sh", "-c", "cargo build --release && tests/run.sh"]
