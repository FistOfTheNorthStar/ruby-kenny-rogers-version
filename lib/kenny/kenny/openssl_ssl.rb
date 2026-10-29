# frozen_string_literal: true
# Kenny: the C half of the openssl gem's TLS (ossl_ssl.c, ossl_ssl_session.c): SSLContext's
# settings and #setup, SSLSocket's handshake, sysread/syswrite and connection details, and
# Session, on Kenny's TLS engine (rustls: crates/kenny-runtime/src/openssl_tls.rs). As in
# CRuby, openssl/ssl.rb (set_params, SSLServer, hostname verification, ...) and
# openssl/buffering.rb (gets, read, puts, ...) of the Ruby the program is built against load
# on top (lib/kenny/openssl.rb requires them after this file).
#
# Certificates are verified as OpenSSL does: the context's X509::Store, verify_depth,
# verify_callback and (for a client with verify_hostname) the hostname check, called by the
# engine during the handshake. TLS 1.2 and 1.3 only; the cipher suites are rustls' (AES-GCM,
# ChaCha20-Poly1305 with ECDHE), so `ciphers=` selects among those.

module OpenSSL
  module PKey
    class DH
      # DH parameters in PEM, as openssl/ssl.rb (its ffdhe2048 group) and SSLContext#tmp_dh=
      # use them: Kenny's TLS does ECDHE key exchange, so they're kept, not used. DH keys
      # themselves stay unsupported.
      def initialize(arg = nil, *)
        unless arg.is_a?(String) && arg.include?("-----BEGIN DH PARAMETERS-----")
          raise NotImplementedError, "OpenSSL::PKey::DH keys are not supported by Kenny (RSA keys are)"
        end
        @__kn_pem = arg
      end

      def to_pem = @__kn_pem
      alias export to_pem
      alias to_s to_pem
      def public? = false
      def private? = false
    end
  end

  module SSL
    class SSLErrorWaitReadable < SSLError
      include IO::WaitReadable
    end

    class SSLErrorWaitWritable < SSLError
      include IO::WaitWritable
    end

    # (OpenSSL 3's values)
    {
      OP_ALL: 0x80000850, OP_ALLOW_CLIENT_RENEGOTIATION: 256, OP_ALLOW_NO_DHE_KEX: 1024,
      OP_ALLOW_UNSAFE_LEGACY_RENEGOTIATION: 262144, OP_CIPHER_SERVER_PREFERENCE: 4194304, OP_CLEANSE_PLAINTEXT: 2,
      OP_CRYPTOPRO_TLSEXT_BUG: 0x80000000, OP_DISABLE_TLSEXT_CA_NAMES: 512, OP_DONT_INSERT_EMPTY_FRAGMENTS: 2048,
      OP_ENABLE_KTLS: 8, OP_ENABLE_MIDDLEBOX_COMPAT: 1048576, OP_EPHEMERAL_RSA: 0, OP_IGNORE_UNEXPECTED_EOF: 128,
      OP_LEGACY_SERVER_CONNECT: 4, OP_MICROSOFT_BIG_SSLV3_BUFFER: 0, OP_MICROSOFT_SESS_ID_BUG: 0,
      OP_MSIE_SSLV2_RSA_PADDING: 0, OP_NETSCAPE_CA_DN_BUG: 0, OP_NETSCAPE_CHALLENGE_BUG: 0,
      OP_NETSCAPE_DEMO_CIPHER_CHANGE_BUG: 0, OP_NETSCAPE_REUSE_CIPHER_CHANGE_BUG: 0, OP_NO_ANTI_REPLAY: 16777216,
      OP_NO_COMPRESSION: 131072, OP_NO_ENCRYPT_THEN_MAC: 524288, OP_NO_RENEGOTIATION: 1073741824,
      OP_NO_SESSION_RESUMPTION_ON_RENEGOTIATION: 65536, OP_NO_SSLv2: 0, OP_NO_SSLv3: 33554432, OP_NO_TICKET: 16384,
      OP_NO_TLSv1: 67108864, OP_NO_TLSv1_1: 268435456, OP_NO_TLSv1_2: 134217728, OP_NO_TLSv1_3: 536870912,
      OP_PKCS1_CHECK_1: 0, OP_PKCS1_CHECK_2: 0, OP_PRIORITIZE_CHACHA: 2097152, OP_SAFARI_ECDHE_ECDSA_BUG: 64,
      OP_SINGLE_DH_USE: 0, OP_SINGLE_ECDH_USE: 0, OP_SSLEAY_080_CLIENT_DH_BUG: 0, OP_SSLREF2_REUSE_CERT_TYPE_BUG: 0,
      OP_TLSEXT_PADDING: 16, OP_TLS_BLOCK_PADDING_BUG: 0, OP_TLS_D5_BUG: 0, OP_TLS_ROLLBACK_BUG: 8388608,
      SSL2_VERSION: 2, SSL3_VERSION: 768, TLS1_VERSION: 769, TLS1_1_VERSION: 770, TLS1_2_VERSION: 771, TLS1_3_VERSION: 772
    }.each { |k, v| const_set(k, v) }

    class SSLContext
      SESSION_CACHE_OFF = 0
      SESSION_CACHE_CLIENT = 1
      SESSION_CACHE_SERVER = 2
      SESSION_CACHE_BOTH = 3
      SESSION_CACHE_NO_AUTO_CLEAR = 128
      SESSION_CACHE_NO_INTERNAL_LOOKUP = 256
      SESSION_CACHE_NO_INTERNAL_STORE = 512
      SESSION_CACHE_NO_INTERNAL = 768

      attr_accessor :cert, :key, :client_ca, :ca_file, :ca_path, :timeout, :verify_mode, :verify_depth,
                    :verify_callback, :cert_store, :extra_chain_cert, :client_cert_cb, :session_id_context,
                    :session_get_cb, :session_new_cb, :session_remove_cb, :renegotiation_cb, :verify_hostname,
                    :keylog_cb, :alpn_protocols, :alpn_select_cb, :npn_protocols, :npn_select_cb, :servername_cb
      alias ssl_timeout timeout
      alias ssl_timeout= timeout=

      # the engine's configuration, built by #setup
      attr_reader :__kn_cfg, :__kn_store

      alias __kn_freeze freeze
      private :__kn_freeze

      # (SSL_CTX_new's options: OpenSSL 3 sets these two)
      def options = @__kn_options || (OP_NO_COMPRESSION | OP_ENABLE_MIDDLEBOX_COMPAT)

      def options=(v)
        @__kn_options = v.nil? ? OP_ALL : Integer(v)
      end

      def ciphers = SSL.__kn_ciphers(@__kn_ciphers, @__kn_suites)

      def ciphers=(v)
        @__kn_ciphers = __kn_cipher_list(v)
      end

      def ciphersuites=(v)
        @__kn_suites = __kn_cipher_list(v)
      end

      def sigalgs=(v)
        @__kn_sigalgs = v
      end

      def client_sigalgs=(v)
        @__kn_client_sigalgs = v
      end

      def ecdh_curves=(v)
        @__kn_groups = v
      end

      def groups=(v)
        @__kn_groups = v
      end

      def tmp_dh=(dh)
        @__kn_tmp_dh = dh
      end

      def security_level = @__kn_security_level || 2

      def security_level=(v)
        @__kn_security_level = Integer(v)
      end

      def enable_fallback_scsv
        @__kn_fallback_scsv = true
        nil
      end

      def session_cache_mode = @__kn_cache_mode || SESSION_CACHE_SERVER

      def session_cache_mode=(v)
        @__kn_cache_mode = Integer(v)
      end

      def session_cache_size = @__kn_cache_size || 20480

      def session_cache_size=(v)
        @__kn_cache_size = Integer(v)
      end

      def session_cache_stats
        { cache_num: 0, connect: 0, connect_good: 0, connect_renegotiate: 0, accept: 0, accept_good: 0,
          accept_renegotiate: 0, cache_hits: 0, cb_hits: 0, cache_misses: 0, cache_full: 0, timeouts: 0 }
      end

      def session_add(_session) = false
      def session_remove(_session) = false
      def flush_sessions(_time = nil) = self

      # add_certificate(cert, key, extra_certs = nil): one more certificate the server can
      # present (the first the client can verify is chosen)
      def add_certificate(cert, key, extra_certs = nil)
        raise TypeError, "wrong argument type #{cert.class} (expected OpenSSL/X509)" unless cert.is_a?(X509::Certificate)
        raise TypeError, "wrong argument type #{key.class} (expected OpenSSL/EVP_PKEY)" unless key.is_a?(PKey::PKey)
        (@__kn_certs ||= []) << [cert, key, Array(extra_certs)]
        self
      end

      # Builds the engine's configuration and freezes the context (an SSLSocket does it);
      # nil when already done.
      def setup
        return nil if frozen?
        min = @__kn_min || 0
        max = @__kn_max || 0
        opts = options
        tls12 = (min == 0 || min <= TLS1_2_VERSION) && (max == 0 || max >= TLS1_2_VERSION) && opts & OP_NO_TLSv1_2 == 0
        tls13 = (min == 0 || min <= TLS1_3_VERSION) && (max == 0 || max >= TLS1_3_VERSION) && opts & OP_NO_TLSv1_3 == 0
        keys = []
        if @cert && @key
          keys << [[@cert, *Array(@extra_chain_cert)].map(&:to_der), __kn_key_der(@key)]
        end
        (@__kn_certs || []).each { |c, k, extra| keys << [[c, *extra].map(&:to_der), __kn_key_der(k)] }
        @__kn_no_protocols = !tls12 && !tls13
        @__kn_cfg = SSL.__kn_config(tls12, tls13, @__kn_ciphers, @__kn_suites, keys, @alpn_protocols&.map(&:to_s),
                                    @verify_mode || VERIFY_NONE, opts & OP_NO_TICKET == 0)
        store = @cert_store
        if @ca_file || @ca_path
          store ||= X509::Store.new
          store.add_file(@ca_file) if @ca_file
          store.add_path(@ca_path) if @ca_path
        end
        @__kn_store = store || X509::Store.new
        __kn_freeze
        true
      end
      alias freeze setup

      def __kn_no_protocols? = @__kn_no_protocols

      private

      def set_minmax_proto_version(min, max)
        @__kn_min = __kn_proto_version(min)
        @__kn_max = __kn_proto_version(max)
        nil
      end

      def __kn_proto_version(v)
        case v
        when nil then 0
        when Integer then v
        when Symbol, String
          name = v.to_s
          SSL.const_defined?("#{name}_VERSION") ? SSL.const_get("#{name}_VERSION") : raise(ArgumentError, "unrecognized version #{v.inspect}")
        else
          raise TypeError, "no implicit conversion of #{v.class} into Integer"
        end
      end

      def __kn_cipher_list(v)
        case v
        when nil then nil
        when Array then v.map { |c| c.is_a?(Array) ? c[0] : c }.join(":")
        else v.to_str
        end
      end

      def __kn_key_der(key)
        key.respond_to?(:private_to_der) ? key.private_to_der : key.to_der
      end
    end

    class SSLSocket
      def initialize(io, ctx = nil)
        ctx = SSLContext.new if ctx.nil?
        raise TypeError, "wrong argument type #{ctx.class} (expected OpenSSL/SSL_CTX)" unless ctx.is_a?(SSLContext)
        raise TypeError, "wrong argument type #{io.class} (expected File)" unless io.is_a?(IO)
        ctx.setup
        @io = io
        @context = ctx
        @sync_close = false
        @hostname = nil
        super()
      end

      def hostname=(name)
        raise TypeError, "no implicit conversion of #{name.class} into String" unless name.nil? || name.is_a?(String)
        @hostname = name
      end

      def connect = __kn_start(false, false, true)
      def connect_nonblock(exception: true) = __kn_start(false, true, exception)
      def accept = __kn_start(true, false, true)
      def accept_nonblock(exception: true) = __kn_start(true, true, exception)

      def sysread(len, buf = nil) = __kn_read(len, buf, false, true)
      def syswrite(str) = __kn_write(str, false, true)

      def cert
        @__kn_tls ? @context.cert : nil
      end

      def peer_cert = __kn_peer_chain&.first

      def peer_cert_chain
        chain = __kn_peer_chain
        return nil unless chain
        # (on a server, SSL_get_peer_cert_chain leaves out the client's own certificate)
        @__kn_server ? chain.drop(1) : chain
      end

      def ssl_version = __kn_info&.at(0) || (@context.__kn_cfg && "TLSv1.3")
      def cipher = __kn_info&.at(1)
      def alpn_protocol = __kn_info&.at(2)
      def npn_protocol = nil
      def session_reused? = __kn_info&.at(3) || false
      def state = @__kn_established ? "SSLOK" : "PINIT"
      def pending = @__kn_tls ? @__kn_tls.pending : 0
      def verify_result = @__kn_verify_result || 0
      def tmp_key = nil
      def group = nil
      def sigalg = nil
      def peer_sigalg = nil
      def finished_message = nil
      def peer_finished_message = nil

      def client_ca
        return [] unless @__kn_server
        ca = @context.client_ca
        ca.nil? ? [] : Array(ca).map { |c| c.respond_to?(:subject) ? c.subject : c }
      end

      def session=(sess)
        raise TypeError, "wrong argument type #{sess.class} (expected OpenSSL/SSL_SESSION)" unless sess.is_a?(Session)
        @__kn_sessions = sess.__kn_sessions
        sess
      end

      def export_keying_material(label, length, context = nil)
        raise SSLError, "SSL session is not started yet" unless @__kn_tls
        @__kn_tls.export_keying_material(label.to_str, Integer(length), context)
      end

      # Called by the engine with the peer's certificates (DER, the peer's own first): nil
      # to go on, or the X509 error code that ends the handshake (OpenSSL's
      # ossl_ssl_verify_callback with the context's verify_callback).
      def __kn_verify(ders)
        certs = ders.map { |d| X509::Certificate.new(d) }
        @__kn_peer_certs = certs
        vcb = @context.verify_callback
        host = !@__kn_server && @context.verify_hostname && @hostname
        sctx = X509::StoreContext.new(@context.__kn_store, certs[0], certs.drop(1))
        sctx.__kn_depth = @context.verify_depth if @context.verify_depth
        sctx.__kn_verify_callback = proc do |ok, ctx|
          if ok && host && ctx.error_depth == 0 && !SSL.verify_certificate_identity(ctx.current_cert, host)
            ctx.error = X509::V_ERR_HOSTNAME_MISMATCH
            ok = false
          end
          if vcb
            begin
              vcb.call(ok, ctx)
            rescue Exception # rubocop:disable Lint/RescueException
              warn "exception in verify_callback is ignored", uplevel: 0
              false
            end
          else
            ok
          end
        end
        ok = sctx.verify
        @__kn_verify_result = sctx.error
        @__kn_verify_error = sctx.error_string
        return nil if ok || (@context.verify_mode || VERIFY_NONE) & VERIFY_PEER == 0
        sctx.error.nonzero? || 1
      end

      # Called by the engine (a server) with the ClientHello's server name and ALPN
      # protocols: [the context's configuration, the protocol chosen or nil], after the
      # servername_cb and alpn_select_cb.
      def __kn_hello(server_name, protocols)
        if server_name && (cb = @context.servername_cb)
          ctx = cb.call([self, server_name])
          if ctx.is_a?(SSLContext)
            ctx.setup
            @__kn_first_context = @context
            @context = ctx
          end
        end
        selected = nil
        if (cb = @context.alpn_select_cb) && !protocols.empty?
          selected = cb.call(protocols)
          unless selected.nil? || protocols.include?(selected)
            raise SSLError, "Selected protocol was not offered by the client"
          end
        end
        [@context.__kn_cfg, selected]
      end

      private

      def __kn_start(server, nonblock, exception)
        fn = server ? "SSL_accept" : "SSL_connect"
        unless @__kn_tls
          if @context.__kn_no_protocols?
            raise SSLError, "#{fn} returned=1 errno=0 peeraddr=#{__kn_peeraddr} state=error: no protocols available"
          end
          @__kn_server = server
          cfg = @context.__kn_cfg
          @__kn_tls = if server
            SSL.__kn_server(cfg, @io.fileno, self)
          else
            SSL.__kn_client(cfg, @io.fileno, self, @hostname, __kn_peer_ip, @__kn_sessions)
          end
        end
        r = @__kn_tls.handshake(nonblock)
        case r
        when true
          __kn_established unless @__kn_established
          self
        when :wait_readable, :wait_writable
          return r unless exception
          raise SSLErrorWaitReadable, "read would block" if r == :wait_readable
          raise SSLErrorWaitWritable, "write would block"
        else
          msg = r[0]
          msg = "#{msg} (#{@__kn_verify_error})" if msg == "certificate verify failed" && @__kn_verify_error
          raise SSLError, "#{fn} returned=1 errno=0 peeraddr=#{__kn_peeraddr} state=error: #{msg}"
        end
      end

      def __kn_established
        @__kn_established = true
        info = __kn_info
        version = info&.at(0)
        @__kn_session = [Time.now, OpenSSL::Random.random_bytes(32), @context.timeout || (version == "TLSv1.3" ? 7200 : 300)]
        mode = @context.session_cache_mode
        cb = @context.session_new_cb
        if cb && mode & (@__kn_server ? SSLContext::SESSION_CACHE_SERVER : SSLContext::SESSION_CACHE_CLIENT) != 0
          cb.call([self, Session.new(self)])
        end
      end

      def __kn_read(len, buf, nonblock, exception)
        len = Integer(len)
        raise ArgumentError, "negative length #{len} given" if len < 0
        if buf
          buf = buf.to_str
          raise FrozenError, "can't modify frozen String: #{buf.inspect}" if buf.frozen?
        end
        if len == 0
          return buf ? buf.replace("".b) : "".b
        end
        unless @__kn_tls
          warn "SSL session is not started yet." if $VERBOSE
          return nonblock ? @io.read_nonblock(len, buf, exception: exception) : @io.sysread(len, buf)
        end
        r = @__kn_tls.read(len, nonblock)
        case r
        when String
          buf ? buf.replace(r) : r
        when nil
          return nil if nonblock && !exception
          raise EOFError, "end of file reached"
        when :wait_readable, :wait_writable
          return r unless exception
          raise SSLErrorWaitReadable, "read would block" if r == :wait_readable
          raise SSLErrorWaitWritable, "write would block"
        else
          raise SSLError, "SSL_read: #{r[0]}"
        end
      end

      def __kn_write(str, nonblock, exception)
        str = str.to_s
        unless @__kn_tls
          warn "SSL session is not started yet." if $VERBOSE
          return nonblock ? @io.write_nonblock(str, exception: exception) : @io.syswrite(str)
        end
        return 0 if str.empty?
        r = @__kn_tls.write(str, nonblock)
        case r
        when Integer then r
        when :wait_readable, :wait_writable
          return r unless exception
          raise SSLErrorWaitReadable, "read would block" if r == :wait_readable
          raise SSLErrorWaitWritable, "write would block"
        else
          raise SSLError, "SSL_write: #{r[0]}"
        end
      end

      def sysread_nonblock(len, buf = nil, exception: true) = __kn_read(len, buf, true, exception)
      def syswrite_nonblock(str, exception: true) = __kn_write(str, true, exception)

      def stop
        @__kn_tls&.close_notify
        nil
      end

      def __kn_info = @__kn_tls&.info

      def __kn_peer_chain
        return @__kn_peer_certs if @__kn_peer_certs
        ders = __kn_info&.at(5)
        @__kn_peer_certs = ders&.map { |d| X509::Certificate.new(d) }
      end

      def __kn_peer_ip
        @io.remote_address.ip_address
      rescue StandardError
        nil
      end

      def __kn_peeraddr
        a = @io.peeraddr
        "#{a[3]}:#{a[1]}"
      rescue StandardError
        "(null)"
      end

      # [time, id, timeout] of the session established (Session.new)
      def __kn_session_info = @__kn_session
      def __kn_tls_state = @__kn_tls
    end

    class Session
      class SessionError < OpenSSLError; end

      attr_reader :time, :timeout, :id, :__kn_sessions

      def initialize(arg)
        unless arg.is_a?(SSLSocket)
          raise SessionError, "SSL session serialization (PEM/DER) is not supported by Kenny"
        end
        info = arg.__send__(:__kn_session_info)
        raise SessionError, "no session available" unless info
        @time, @id, @timeout = info
        @__kn_sessions = arg.__send__(:__kn_tls_state)&.sessions
      end

      def time=(t)
        @time = t.is_a?(Time) ? t : Time.at(Integer(t))
      end

      def timeout=(n)
        @timeout = Integer(n)
      end

      def ==(other) = other.is_a?(Session) && id == other.id

      def to_der = raise(SessionError, "SSL session serialization is not supported by Kenny")
      def to_pem = raise(SessionError, "SSL session serialization is not supported by Kenny")
      def to_text = raise(SessionError, "SSL session serialization is not supported by Kenny")
    end
  end
end
