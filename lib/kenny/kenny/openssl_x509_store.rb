# frozen_string_literal: true
# Kenny: OpenSSL::X509::Store and StoreContext (ossl_x509store.c), with certificate
# verification following OpenSSL 3.6's crypto/x509/x509_vfy.c: build_chain (trusted-first
# issuer lookup, self-signed trust anchors, the untrusted chain), check_extensions (CA flags,
# purposes, path lengths, critical extensions), CRL checks, internal_verify (signatures and
# validity times), with the verify callback called where OpenSSL calls it and OpenSSL's error
# codes. Name constraints and policies aren't checked.

module Kenny
  # x509v3_cache_extensions: what verification looks at in a certificate
  class X509Info
    KU_DIGSIG = 0x80
    KU_NONREP = 0x40
    KU_KEYENC = 0x20
    KU_KEYAGREE = 0x08
    KU_CERTSIGN = 0x04
    KU_CRLSIGN = 0x02
    XKU_BITS = { "1.3.6.1.5.5.7.3.1" => 0x1, "1.3.6.1.5.5.7.3.2" => 0x2, "1.3.6.1.5.5.7.3.4" => 0x4, "1.3.6.1.5.5.7.3.3" => 0x8,
                 "2.16.840.1.113730.4.1" => 0x10, "1.3.6.1.4.1.311.10.3.3" => 0x10, "1.3.6.1.5.5.7.3.9" => 0x20,
                 "1.3.6.1.5.5.7.3.8" => 0x40, "1.3.6.1.5.5.7.3.10" => 0x80, "2.5.29.37.0" => 0x100 }.freeze
    # X509_supported_extension
    SUPPORTED = %w[2.16.840.1.113730.1.1 2.5.29.15 2.5.29.17 2.5.29.19 2.5.29.32 2.5.29.37 2.5.29.36 1.3.6.1.5.5.7.1.14
                   2.5.29.30 2.5.29.33 2.5.29.54 1.3.6.1.5.5.7.1.7 1.3.6.1.5.5.7.1.8].freeze
    D = Kenny::DER

    attr_reader :cert, :der, :subject, :issuer, :spki, :ku, :xku, :ns, :pathlen, :skid, :akid, :not_before, :not_after
    attr_reader :serial, :ext_oids, :ext_crit

    def initialize(cert)
      @cert = cert
      @der = cert.to_der
      @subject = OpenSSL::X509::Name.new(cert.__kenny_subject_der)
      @issuer = OpenSSL::X509::Name.new(cert.__kenny_issuer_der)
      @spki = cert.__kenny_spki
      @serial = cert.__kenny_serial_i
      @not_before = cert.__kenny_not_before
      @not_after = cert.__kenny_not_after
      @v1 = cert.version == 0
      @bcons = @ca = @critical = @invalid = false
      @pathlen = -1
      @ku = @xku = @ns = @skid = @akid = nil
      @ext_oids = []
      @ext_crit = {}
      cert.__kenny_ext_ders.each { |d| __ext(OpenSSL::X509::Extension.new(d)) }
      @si = @subject.cmp(@issuer) == 0
      @ss = @si && check_akid(self) == 0 && sig_alg_match(self) == 0
    end

    def ss? = @ss
    def si? = @si
    def v1? = @v1
    def bcons? = @bcons
    def ca_flag? = @ca
    def critical? = @critical
    def invalid? = @invalid

    def ku_reject(bits) = !@ku.nil? && @ku & bits == 0
    def xku_reject(bits) = !@xku.nil? && @xku & bits == 0
    def ns_reject(bits) = !@ns.nil? && @ns & bits == 0

    # check_ca: 0 not a CA; 1 basicConstraints CA; 3 v1 self-signed; 4 keyUsage; 5 nsCertType CA
    def check_ca
      return 0 if ku_reject(KU_CERTSIGN)
      return @ca ? 1 : 0 if @bcons
      return 3 if @v1 && @ss
      return 4 unless @ku.nil?
      return 5 if @ns && @ns & 0x07 != 0
      0
    end

    # X509_check_akid(self as issuer, akid of the subject)
    def check_akid(subject)
      a = subject.akid or return 0
      keyid, dirname, serial = a
      return 30 if keyid && @skid && keyid != @skid
      return 31 if serial && serial != @serial
      return 31 if dirname && OpenSSL::X509::Name.new(dirname).cmp(@issuer) != 0
      0
    end

    # check_sig_alg_match(issuer key, subject)
    def sig_alg_match(subject)
      return 24 unless @spki
      alg = subject.cert.__kenny_tbs_alg or return 76
      oid, = Kenny::X509Util.alg_id(alg)
      algs = Kenny::X509Util::SIG_ALGS[oid] or return 76
      key_oid, = Kenny::X509Util.spki_parts(@spki)
      return 0 if key_oid == algs[1] || (algs[1] == "1.2.840.113549.1.1.10" && key_oid == "1.2.840.113549.1.1.1")
      77
    end

    # ossl_x509_likely_issued(self, subject)
    def likely_issued(subject)
      return 29 if @subject.cmp(subject.issuer) != 0
      r = check_akid(subject)
      return r if r != 0
      sig_alg_match(subject)
    end

    def signing_allowed(_subject)
      ku_reject(KU_CERTSIGN) ? 32 : 0
    end

    private

    def __ext(e)
      oid = e.__kenny_dotted
      @ext_oids << oid
      @ext_crit[oid] = e.critical?
      @critical = true if e.critical? && !SUPPORTED.include?(oid)
      c = e.value_der
      case oid
      when "2.5.29.19"
        @bcons = true
        kids = D.children(D.unwrap(c)[1])
        if kids[0] && kids[0][0] == 1
          @ca = kids.shift[1].getbyte(0) != 0
        end
        if kids[0]
          v = D.int_value(kids[0][1])
          v < 0 ? @invalid = true : @pathlen = v
        end
      when "2.5.29.15"
        b = D.unwrap(c)[1]
        @ku = (b.getbyte(1) || 0) | ((b.getbyte(2) || 0) << 8)
      when "2.5.29.37"
        @xku = 0
        D.children(D.unwrap(c)[1]).each { |_, oc| @xku |= XKU_BITS.fetch(D.oid_dotted(oc), 0) }
      when "2.16.840.1.113730.1.1"
        b = D.unwrap(c)[1]
        @ns = b.getbyte(1) || 0
      when "2.5.29.14"
        @skid = D.unwrap(c)[1]
      when "2.5.29.35"
        keyid = dirname = serial = nil
        D.children(D.unwrap(c)[1]).each do |b, kc|
          case b
          when 0x80 then keyid = kc
          when 0xa1 then dirname ||= D.children(kc).find { |g,| g == 0xa4 }&.then { |_, gc| D.children(gc)[0][2] }
          when 0x82 then serial = D.int_value(kc)
          end
        end
        @akid = [keyid, dirname, serial]
      end
    rescue OpenSSL::ASN1::ASN1Error, NoMethodError, TypeError
      @invalid = true
    end
  end

  # the trusted certificates and CRLs of a Store: given ones, files (the default one read
  # when first needed), and hashed directories looked up by subject
  class X509Trust
    def initialize
      @certs = {}
      @crls = []
      @files = []
      @dirs = []
      @loaded_dirs = {}
    end

    def add_cert(cert)
      info = Kenny::X509Info.new(cert)
      list = (@certs[info.subject.__kenny_canon] ||= [])
      list << info unless list.any? { |i| i.der == info.der }
    end

    def add_crl(crl) = @crls << crl

    # X509_load_cert_crl_file: every certificate and CRL in a PEM file (or one DER certificate)
    def load_file(path, lazy: false)
      if lazy
        @files << path
        return true
      end
      data = File.binread(path)
      n = 0
      blocks = Kenny::X509Util.pem_blocks(data)
      if blocks.empty?
        begin
          add_cert(OpenSSL::X509::Certificate.new(data))
          n += 1
        rescue OpenSSL::X509::CertificateError
          nil
        end
      end
      blocks.each do |label, der|
        next unless der
        case label
        when "CERTIFICATE", "X509 CERTIFICATE", "TRUSTED CERTIFICATE"
          begin
            add_cert(OpenSSL::X509::Certificate.new(Kenny::DER.first(der)[2]))
            n += 1
          rescue OpenSSL::X509::CertificateError, OpenSSL::ASN1::ASN1Error
            nil
          end
        when "X509 CRL"
          add_crl(OpenSSL::X509::CRL.new(der))
          n += 1
        end
      end
      n
    end

    def add_dir(path) = @dirs.concat(path.split(File::PATH_SEPARATOR))

    def load_pending
      until @files.empty?
        f = @files.shift
        begin
          load_file(f)
        rescue SystemCallError, IOError
          nil
        end
      end
    end

    # X509_STORE_CTX_get_by_subject: the cached certificates with that subject, else those the
    # hashed directories have (<hash>.<n>)
    def by_subject(name)
      load_pending
      canon = name.__kenny_canon
      list = @certs[canon]
      return list if list && !list.empty?
      h = format("%08x", name.hash)
      @dirs.each do |dir|
        key = [dir, h]
        next if @loaded_dirs[key]
        @loaded_dirs[key] = true
        i = 0
        while File.exist?(path = File.join(dir, "#{h}.#{i}"))
          begin
            load_file(path)
          rescue SystemCallError, IOError, OpenSSL::OpenSSLError
            nil
          end
          i += 1
        end
      end
      @certs[canon] || []
    end

    def crls_for(name)
      load_pending
      h = format("%08x", name.hash)
      @dirs.each do |dir|
        key = [dir, h, :crl]
        next if @loaded_dirs[key]
        @loaded_dirs[key] = true
        i = 0
        while File.exist?(path = File.join(dir, "#{h}.r#{i}"))
          begin
            load_file(path)
          rescue SystemCallError, IOError, OpenSSL::OpenSSLError
            nil
          end
          i += 1
        end
      end
      @crls.select { |c| OpenSSL::X509::Name.new(c.__kenny_issuer_der).cmp(name) == 0 }
    end
  end

  # one X509_verify_cert run (a StoreContext's state)
  class X509Verify
    TRUSTED = 1
    REJECTED = 2
    UNTRUSTED = 3
    F = OpenSSL::X509

    attr_accessor :error, :error_depth, :current_cert, :current_crl, :chain, :flags, :purpose, :trust, :depth, :check_time
    attr_accessor :callback, :ctx, :done

    def initialize(trust_store, cert, untrusted, flags, purpose, trust, depth, time)
      @store = trust_store
      @cert = cert
      @untrusted = untrusted
      @flags = flags
      @purpose = purpose
      @trust = trust
      @depth = depth
      @check_time = time
      @error = 0
      @error_depth = 0
      @current_cert = nil
      @current_crl = nil
      @chain = nil
      @infos = {}
    end

    def info(cert)
      @infos[cert.to_der] ||= Kenny::X509Info.new(cert)
    end

    # the verify callback (ossl_verify_cb_call; x509store_verify_cb)
    def cb(ok)
      proc = @callback
      return ok if proc.nil?
      view = OpenSSL::X509::StoreContext.allocate
      view.__send__(:__kenny_view, self)
      ret = false
      begin
        ret = proc.call(ok, view)
      rescue Exception => e # rubocop:disable Lint/RescueException
        raise if e.is_a?(SystemExit) || e.is_a?(Interrupt) || e.is_a?(NoMemoryError)
        loc = caller_locations.find { |l| !l.path.to_s.include?("kenny/openssl") }
        if $VERBOSE != nil
          warn(loc ? "#{loc.path}:#{loc.lineno}: warning: exception in verify_callback is ignored" : "warning: exception in verify_callback is ignored")
        end
        ret = false
      ensure
        view.__send__(:__kenny_view, nil)
      end
      if ret == true
        @error = 0
        true
      else
        @error = 28 if @error == 0
        false
      end
    end

    # verify_cb_cert
    def fail(cert, depth, err)
      depth = @error_depth if depth < 0
      @error_depth = depth
      @current_cert = cert || @chain[depth].cert
      @error = err if err != 0
      cb(false)
    end

    def has?(flag) = @flags & flag != 0

    def now = @check_time || Time.now

    # ossl_x509_check_cert_time; depth < 0: just whether the times are good
    def check_time(i, depth)
      return true if !has?(F::V_FLAG_USE_CHECK_TIME) && has?(F::V_FLAG_NO_CHECK_TIME)
      t = now
      nb = i.not_before
      na = i.not_after
      bad_nb = nb.nil? ? 13 : nb > t ? 9 : nil
      return false if bad_nb && depth < 0
      return false if bad_nb && !fail(i.cert, depth, bad_nb)
      bad_na = na.nil? ? 14 : na <= t ? 10 : nil
      return false if bad_na && depth < 0
      return false if bad_na && !fail(i.cert, depth, bad_na)
      true
    end

    def issued?(x, issuer) = issuer.likely_issued(x) == 0

    # X509_STORE_CTX_get1_issuer: a matching store certificate valid now, else the one that
    # expires last
    def trusted_issuer(x)
      best = nil
      @store.by_subject(x.issuer).each do |c|
        next unless issued?(x, c)
        return c if check_time(c, -1)
        best = c if best.nil? || (c.not_after && best.not_after && c.not_after > best.not_after)
      end
      best
    end

    # find_issuer in the untrusted certificates
    def find_issuer(list, x)
      rv = nil
      list.each do |c|
        next unless issued?(x, c) && ((x.si? && @chain.size == 1) || @chain.none? { |y| y.der == c.der })
        return c if check_time(c, -1)
        rv = c if rv.nil? || (c.not_after && rv.not_after && c.not_after > rv.not_after)
      end
      rv
    end

    def trusted_cert?(i)
      i.ss? && @trust != F::TRUST_OCSP_SIGN && @trust != F::TRUST_OCSP_REQUEST
    end

    def check_trust(num_untrusted)
      num = @chain.size
      (num_untrusted...num).each do |i|
        return TRUSTED if trusted_cert?(@chain[i])
      end
      if num_untrusted < num
        return has?(F::V_FLAG_PARTIAL_CHAIN) ? TRUSTED : UNTRUSTED
      end
      if num_untrusted == num && has?(F::V_FLAG_PARTIAL_CHAIN)
        leaf = @chain[0]
        mx = @store.by_subject(leaf.subject).find { |c| c.der == leaf.der } or return UNTRUSTED
        @chain[0] = mx
        @num_untrusted = 0
        return TRUSTED
      end
      UNTRUSTED
    end

    def build_chain
      untrusted = @untrusted.map { |c| info(c) }
      search_untrusted = !@untrusted.empty?
      search_trusted = true
      trust = UNTRUSTED
      max_depth = @depth + 1
      while search_untrusted || search_trusted
        num = @chain.size
        @error_depth = num - 1
        if search_trusted
          curr = @chain[num - 1]
          issuer = num > max_depth ? nil : trusted_issuer(curr)
          ok = !issuer.nil?
          if ok
            self_signed = curr.ss?
            if !self_signed
              @chain << issuer
              self_signed = issuer.ss?
            elsif curr.der != issuer.der
              ok = false
            else
              num -= 1
              @num_untrusted = num
              @chain[num] = issuer
            end
            if ok
              search_untrusted = false
              trust = check_trust(@num_untrusted)
              break if trust != UNTRUSTED
              next unless self_signed
            end
          end
          break unless search_untrusted
        end
        if search_untrusted
          num = @chain.size
          curr = @chain[num - 1]
          issuer = curr.ss? || num > max_depth ? nil : find_issuer(untrusted, curr)
          if issuer.nil?
            search_untrusted = false
            search_trusted = true
            next
          end
          untrusted.delete(issuer)
          @chain << issuer
          @num_untrusted += 1
        end
      end
      num = @chain.size
      trust = check_trust(num) if num <= max_depth && trust == UNTRUSTED && num == @num_untrusted
      return true if trust == TRUSTED
      return false if trust == REJECTED
      num = @chain.size
      return false if num > max_depth && !fail(nil, num - 1, 22)
      if @chain[num - 1].ss?
        return fail(nil, num - 1, num == 1 ? 18 : 19)
      end
      fail(nil, num - 1, @num_untrusted < num ? 2 : 20)
    end

    # X509_check_purpose's per-purpose checks (crypto/x509/v3_purp.c)
    def purpose_ok(x, id, ca)
      case id
      when F::PURPOSE_SSL_CLIENT
        return 0 if x.xku_reject(0x2)
        return ssl_ca(x) if ca
        return 0 if x.ku_reject(X509Info::KU_DIGSIG | X509Info::KU_KEYAGREE)
        return 0 if x.ns_reject(0x80)
        1
      when F::PURPOSE_SSL_SERVER, F::PURPOSE_NS_SSL_SERVER
        return 0 if x.xku_reject(0x1 | 0x10)
        return ssl_ca(x) if ca
        return 0 if x.ns_reject(0x40)
        return 0 if x.ku_reject(X509Info::KU_DIGSIG | X509Info::KU_KEYENC | X509Info::KU_KEYAGREE)
        return 0 if id == F::PURPOSE_NS_SSL_SERVER && x.ku_reject(X509Info::KU_KEYENC)
        1
      when F::PURPOSE_SMIME_SIGN, F::PURPOSE_SMIME_ENCRYPT
        r = smime(x, ca)
        return r if r == 0 || ca
        bits = id == F::PURPOSE_SMIME_SIGN ? X509Info::KU_DIGSIG | X509Info::KU_NONREP : X509Info::KU_KEYENC
        x.ku_reject(bits) ? 0 : r
      when F::PURPOSE_CRL_SIGN
        if ca
          r = x.check_ca
          return r == 2 ? 0 : r
        end
        x.ku_reject(X509Info::KU_CRLSIGN) ? 0 : 1
      when F::PURPOSE_OCSP_HELPER then ca ? x.check_ca : 1
      when F::PURPOSE_TIMESTAMP_SIGN
        return x.check_ca if ca
        return 0 if x.ext_oids.include?("2.5.29.37") && !x.ext_crit["2.5.29.37"]
        return 0 unless x.xku == 0x40
        return 0 if x.ku && ((x.ku & ~(X509Info::KU_NONREP | X509Info::KU_DIGSIG)) != 0 || x.ku & (X509Info::KU_NONREP | X509Info::KU_DIGSIG) == 0)
        1
      when 10
        # (code signing)
        return x.check_ca if ca
        return 0 if x.ku.nil? || x.ku & X509Info::KU_DIGSIG == 0 || x.ku & (X509Info::KU_CERTSIGN | X509Info::KU_CRLSIGN) != 0
        return 0 if x.xku.nil? || x.xku & 0x8 == 0 || x.xku & (0x100 | 0x1) != 0
        1
      when F::PURPOSE_ANY then 1
      else -1
      end
    end

    def ssl_ca(x)
      r = x.check_ca
      return 0 if r == 0
      r != 5 || x.ns & 0x04 != 0 ? 1 : 0
    end

    def smime(x, ca)
      return 0 if x.xku_reject(0x4)
      if ca
        r = x.check_ca
        return 0 if r == 0
        return r != 5 || x.ns & 0x02 != 0 ? r : 0
      end
      return x.ns & 0x20 != 0 ? 1 : x.ns & 0x80 != 0 ? 2 : 0 if x.ns
      1
    end

    def check_purpose(x, depth, must_be_ca)
      r = purpose_ok(x, @purpose, must_be_ca > 0)
      return true if r == 1 || (r != 0 && !has?(F::V_FLAG_X509_STRICT))
      fail(x.cert, depth, 26)
    end

    def check_extensions
      must_be_ca = -1
      plen = 0
      num = @chain.size
      @chain.each_with_index do |x, i|
        return false if x.invalid? && !fail(x.cert, i, 41)
        return false if !has?(F::V_FLAG_IGNORE_CRITICAL) && x.critical? && !fail(x.cert, i, 34)
        ret = x.check_ca
        bad = must_be_ca == -1 ? has?(F::V_FLAG_X509_STRICT) && ret != 1 && ret != 0 : ret == 0 || ((i + 1 < num || has?(F::V_FLAG_X509_STRICT)) && ret != 1)
        return false if bad && !fail(x.cert, i, 79)
        return false if @purpose > 0 && !check_purpose(x, i, must_be_ca)
        return false if i > 1 && x.pathlen != -1 && plen > x.pathlen && !fail(x.cert, i, 25)
        plen += 1 if i > 0 && !x.si?
        must_be_ca = 1
      end
      true
    end

    # check_revocation, simplified: a CRL from the certificate's issuer, signed by it, current,
    # not listing the certificate
    def check_revocation
      return true unless has?(F::V_FLAG_CRL_CHECK)
      last = has?(F::V_FLAG_CRL_CHECK_ALL) ? @chain.size - 1 : 0
      (0..last).each do |i|
        @error_depth = i
        x = @chain[i]
        @current_cert = x.cert
        issuer = @chain[i + 1] || (x.ss? ? x : nil)
        crls = @store.crls_for(x.issuer)
        if crls.empty? || issuer.nil?
          @error = 3
          return false unless cb(false)
          next
        end
        crl = crls.find { |c| crl_signed?(c, issuer) } || crls[0]
        @current_crl = crl
        unless crl_signed?(crl, issuer)
          @error = 8
          return false unless cb(false)
        end
        unless has?(F::V_FLAG_NO_CHECK_TIME) && !has?(F::V_FLAG_USE_CHECK_TIME)
          if crl.__kenny_last_update.nil? || crl.__kenny_last_update > now
            @error = 11
            return false unless cb(false)
          end
          if crl.__kenny_next_update && crl.__kenny_next_update <= now
            @error = 12
            return false unless cb(false)
          end
        end
        if issuer.ku_reject(X509Info::KU_CRLSIGN)
          @error = 35
          return false unless cb(false)
        end
        if crl.__kenny_revoked_serials.include?(x.serial)
          @error = 23
          return false unless cb(false)
        end
        @current_crl = nil
      end
      true
    end

    def crl_signed?(crl, issuer)
      Kenny::X509Util.verify(issuer.spki, crl.__kenny_sig_alg, crl.__kenny_tbs_bytes, crl.__kenny_sig, OpenSSL::X509::CRLError)
    rescue OpenSSL::OpenSSLError, NotImplementedError
      false
    end

    def internal_verify
      n = @chain.size - 1
      xi = @chain[n]
      xs = xi
      @error_depth = n
      if xi.likely_issued(xi) != 0 && !has?(F::V_FLAG_PARTIAL_CHAIN)
        if n > 0
          n -= 1
          @error_depth = n
          xs = @chain[n]
        else
          return false unless fail(xi.cert, 0, 21)
        end
      end
      while n >= 0
        if !xs.equal?(xi) || (has?(F::V_FLAG_CHECK_SS_SIGNATURE) && xi.ss?)
          issuer_depth = n + (xs.equal?(xi) ? 0 : 1)
          ret = xs.equal?(xi) && !xi.ca_flag? ? 0 : xi.signing_allowed(xs)
          return false if ret != 0 && !fail(xi.cert, issuer_depth, ret)
          if xi.spki.nil?
            return false unless fail(xi.cert, issuer_depth, 6)
          else
            good = begin
              xs.cert.__kenny_sig_alg == xs.cert.__kenny_tbs_alg &&
                Kenny::X509Util.verify(xi.spki, xs.cert.__kenny_sig_alg, xs.cert.tbs_bytes, xs.cert.__kenny_sig, OpenSSL::X509::CertificateError)
            rescue OpenSSL::OpenSSLError
              false
            end
            return false if !good && !fail(xs.cert, n, 7)
          end
        end
        return false unless check_time(xs, n)
        @current_cert = xs.cert
        @error_depth = n
        return false unless cb(true)
        n -= 1
        if n >= 0
          xi = xs
          xs = @chain[n]
        end
      end
      true
    end

    # X509_verify_cert: 1, 0, or -1 (StoreError)
    def run
      if @cert.nil?
        @error = 69
        return [-1, "no cert set for us to verify"]
      end
      if @chain
        @error = 69
        return [-1, "called a function you should not call"]
      end
      @chain = [info(@cert)]
      @num_untrusted = 1
      ok = build_chain && check_extensions && check_revocation && internal_verify
      @error = 1 if !ok && @error == 0
      [ok ? 1 : 0, nil]
    end

    def chain_certs = @chain&.map(&:cert)
  end
end

module OpenSSL
  module X509
    class Store
      attr_reader :verify_callback, :error, :error_string, :chain

      def initialize(*args)
        warn("OpenSSL::X509::Store.new does not take any arguments") unless args.empty?
        @verify_callback = nil
        @error = nil
        @error_string = nil
        @chain = nil
        @__kenny = Kenny::X509Trust.new
        @__kenny_param = { flags: 0, purpose: 0, trust: 0, time: nil }
      end

      def verify_callback=(cb)
        @verify_callback = cb
      end

      def flags=(flags)
        @__kenny_param[:flags] |= Integer(flags)
      end

      def purpose=(purpose)
        p = Integer(purpose)
        @__kenny_param[:purpose] = p if (1..10).cover?(p)
      end

      def trust=(trust)
        t = Integer(trust)
        @__kenny_param[:trust] = t if (1..8).cover?(t)
      end

      def time=(time)
        @__kenny_param[:time] = time
      end

      def add_file(path)
        path = File.path(path)
        n = begin
          @__kenny.load_file(path)
        rescue SystemCallError, IOError
          raise StoreError, "X509_LOOKUP_load_file: BIO lib"
        end
        raise StoreError, "X509_LOOKUP_load_file: no certificate or crl found" if n == 0
        self
      end

      def add_path(path)
        @__kenny.add_dir(File.path(path))
        self
      end

      # X509_STORE_set_default_paths: the default file and directory (SSL_CERT_FILE and
      # SSL_CERT_DIR override them); Kenny reads them when a verification first needs them
      def set_default_paths
        file = ENV[DEFAULT_CERT_FILE_ENV] || DEFAULT_CERT_FILE
        @__kenny.load_file(file, lazy: true) if File.file?(file)
        dir = ENV[DEFAULT_CERT_DIR_ENV] || DEFAULT_CERT_DIR
        @__kenny.add_dir(dir)
        self
      end

      def add_cert(cert)
        raise TypeError, "wrong argument type #{Kenny::DER.class_name(cert)} (expected OpenSSL/X509)" unless cert.is_a?(Certificate)
        @__kenny.add_cert(cert)
        self
      end

      def add_crl(crl)
        raise TypeError, "wrong argument type #{Kenny::DER.class_name(crl)} (expected OpenSSL/X509/CRL)" unless crl.is_a?(CRL)
        @__kenny.add_crl(crl)
        self
      end

      def verify(cert, chain = nil, &block)
        ctx = StoreContext.new(self, cert, chain)
        ctx.instance_variable_set(:@verify_callback, block || @verify_callback)
        result = ctx.verify
        @error = ctx.error
        @error_string = ctx.error_string
        @chain = ctx.chain
        result
      end

      private

      def __kenny_trust = @__kenny
      def __kenny_param = @__kenny_param
    end

    class StoreContext
      def initialize(store, cert = nil, chain = nil)
        raise TypeError, "wrong argument type #{Kenny::DER.class_name(store)} (expected OpenSSL/X509/STORE)" unless store.is_a?(Store)
        raise TypeError, "wrong argument type #{Kenny::DER.class_name(cert)} (expected OpenSSL/X509)" unless cert.nil? || cert.is_a?(Certificate)
        untrusted = []
        unless chain.nil?
          ary = Array.try_convert(chain) or raise TypeError, "wrong argument type #{Kenny::DER.class_name(chain)} (expected Array)"
          ary.each { |c| raise OpenSSLError, "object in array not of class ##type##" unless c.is_a?(Certificate) }
          untrusted = ary.map(&:dup)
        end
        param = store.__send__(:__kenny_param)
        # (X509_STORE_CTX_init: the store's parameters over the "default" ones, which add
        # X509_V_FLAG_TRUSTED_FIRST and a depth of 100)
        @__kenny = Kenny::X509Verify.new(store.__send__(:__kenny_trust), cert&.dup, untrusted, param[:flags] | V_FLAG_TRUSTED_FIRST,
                                         param[:purpose], param[:trust], 100, nil)
        @__kenny.ctx = self
        __kenny_purpose_trust
        self.time = param[:time] if param[:time]
        @verify_callback = store.verify_callback
        @cert = cert
      end

      def verify
        st = __kenny_st
        st.callback = @__kn_verify_callback || @verify_callback
        r, msg = st.run
        raise StoreError, "X509_verify_cert: #{msg}" if r < 0
        r == 1
      end

      def chain = __kenny_st.chain_certs

      def error = __kenny_st.error

      def error=(err)
        __kenny_st.error = Integer(err)
      end

      def error_string = X509.__kenny_error_string(__kenny_st.error)
      def error_depth = __kenny_st.error_depth

      def current_cert
        c = __kenny_st.current_cert
        c.nil? && OpenSSL::VERSION < "4" ? Certificate.new : c
      end

      def current_crl = __kenny_st.current_crl

      def flags=(flags)
        __kenny_st.flags |= Integer(flags)
      end

      # (X509_STORE_CTX_set_purpose and _set_trust only fill in unset values)
      def purpose=(purpose)
        p = Integer(purpose)
        st = __kenny_st
        if st.purpose == 0 && (1..10).cover?(p)
          st.purpose = p
          __kenny_purpose_trust
        end
      end

      def trust=(trust)
        t = Integer(trust)
        st = __kenny_st
        st.trust = t if st.trust == 0 && (1..8).cover?(t)
      end

      def time=(time)
        st = __kenny_st
        st.check_time = time.is_a?(Time) ? time : Time.at(Integer(time))
        st.flags |= V_FLAG_USE_CHECK_TIME
      end

      # Kenny's hooks for its SSL layer (lib/kenny/kenny/openssl_ssl.rb): a verify callback
      # that takes the place of the store's, and the verify depth (nil: OpenSSL's 100)
      def __kn_verify_callback=(proc)
        @__kn_verify_callback = proc
      end

      def __kn_depth=(n)
        __kenny_st.depth = n.nil? ? 100 : Integer(n)
      end

      private

      PURPOSE_TRUST = { 1 => 2, 2 => 3, 3 => 3, 4 => 4, 5 => 4, 6 => 1, 7 => 0, 8 => 1, 9 => 8, 10 => 5 }.freeze
      private_constant :PURPOSE_TRUST

      def __kenny_purpose_trust
        st = __kenny_st
        st.trust = PURPOSE_TRUST.fetch(st.purpose, 0) if st.trust == 0
      end

      # the callback's view of a running verification (a StoreContext for the duration of the call)
      def __kenny_view(state)
        @__kenny = state
      end

      def __kenny_st
        @__kenny or raise RuntimeError, "STORE_CTX is out of scope!"
      end
    end
  end
end
