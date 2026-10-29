/* Kenny C-API shim: the parts of the CRuby API that need setjmp/longjmp or varargs, and those
 * built on calling Ruby methods (rb_funcall).
 * Exceptions raised while a C method runs longjmp back to that method's call boundary
 * (kn_call), exactly like CRuby. The longjmp only ever crosses C frames. */
#include "ruby.h"
#include <setjmp.h>
#include <stdarg.h>
#include <stdio.h>
#include <errno.h>

/* implemented in the Rust runtime */
typedef VALUE (*kn_cfunc_caller)(const void *, int, VALUE, int, const VALUE *);
void kn_capi_register_caller(kn_cfunc_caller f);
VALUE kn_capi_funcallv(VALUE recv, ID mid, int argc, const VALUE *argv);
VALUE kn_capi_funcall_with_block(VALUE recv, ID mid, int argc, const VALUE *argv, VALUE blk);
VALUE kn_capi_yield_values(int argc, const VALUE *argv);
VALUE kn_capi_raise(VALUE klass, const char *msg);
VALUE kn_capi_exc_raise(VALUE e);
VALUE kn_capi_take_pending(void);
int kn_capi_pending(void);
VALUE kn_capi_check_type(VALUE v, int t);
VALUE kn_capi_check_frozen(VALUE v);
long kn_capi_num2long(VALUE v, int *ok);
unsigned long kn_capi_num2ulong(VALUE v, int *ok);
unsigned long long kn_capi_big2ull(VALUE v, int *ok);
long long kn_capi_big2ll(VALUE v, int *ok);
double kn_capi_num2dbl(VALUE v, int *ok);
VALUE kn_capi_string_value(VALUE v);
VALUE kn_capi_inspect(VALUE v);
VALUE kn_capi_hash_aref(VALUE h, VALUE k);
VALUE kn_capi_hash_pairs(VALUE h);
VALUE kn_capi_class_new_instance(int argc, const VALUE *argv, VALUE klass);
VALUE kn_capi_const_get(VALUE klass, ID id);
VALUE kn_capi_path2class(const char *path);
VALUE kn_capi_obj_alloc(VALUE klass);
void *kn_capi_check_typeddata(VALUE obj, const rb_data_type_t *type, int *ok);
void kn_capi_warn(const char *msg);
VALUE kn_capi_str_cat(VALUE s, const char *p, long len);
void kn_capi_str_reserve(VALUE s, long extra);
int kn_capi_is_kwhash(VALUE v);
VALUE kn_capi_kwhash(VALUE v);
VALUE kn_capi_proc_new(const void *func, VALUE data2);
void kn_capi_push_block(VALUE b);
void kn_capi_pop_block(void);
VALUE kn_capi_save_throw(void);
void kn_capi_restore_throw(VALUE t);

#define KN_MAX_DEPTH 4096
/* (per OS thread: Kenny's threads run Ruby, and C extension code, in parallel) */
static __thread jmp_buf *kn_jmp_stack[KN_MAX_DEPTH];
static __thread int kn_jmp_depth = 0;

static void kn_unwind(void) __attribute__((noreturn));
static void kn_unwind(void) {
  if (kn_jmp_depth > 0) longjmp(*kn_jmp_stack[kn_jmp_depth - 1], 1);
  fprintf(stderr, "kenny: exception raised outside of any C method call\n");
  abort();
}
#define KN_CHECK(v) do { if ((VALUE)(v) == Qundef) kn_unwind(); } while (0)

typedef VALUE (*f0)(VALUE);
typedef VALUE (*f1)(VALUE, VALUE);
typedef VALUE (*f2)(VALUE, VALUE, VALUE);
typedef VALUE (*f3)(VALUE, VALUE, VALUE, VALUE);
typedef VALUE (*f4)(VALUE, VALUE, VALUE, VALUE, VALUE);
typedef VALUE (*f5)(VALUE, VALUE, VALUE, VALUE, VALUE, VALUE);
typedef VALUE (*f6)(VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE);
typedef VALUE (*f7)(VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE);
typedef VALUE (*f8)(VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE);
typedef VALUE (*f9)(VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE);
typedef VALUE (*f10)(VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE, VALUE);
typedef VALUE (*fv)(int, const VALUE *, VALUE);
typedef VALUE (*fa)(VALUE, VALUE);
typedef VALUE (*fb)(VALUE, VALUE, int, const VALUE *, VALUE);

/* Call C function `f` (arity: a method's argc; -3: a block function, see rb_proc_new, whose
 * args end with the block argument). Returns Qundef when it raised (the exception is
 * pending in the runtime). */
static VALUE kn_call(const void *f, int arity, VALUE self, int argc, const VALUE *a) {
  jmp_buf jb;
  volatile VALUE r = Qundef;
  if (kn_jmp_depth >= KN_MAX_DEPTH) return kn_capi_raise(rb_eRuntimeError, "C call stack too deep");
  kn_jmp_stack[kn_jmp_depth++] = &jb;
  if (setjmp(jb) == 0) {
    switch (arity) {
      case -3: {
        int n = argc - 1;
        r = ((fb)f)(n > 0 ? a[0] : Qnil, self, n, a, a[n]);
        break;
      }
      case -2: r = ((fa)f)(self, rb_ary_new_from_values(argc, a)); break;
      case -1: r = ((fv)f)(argc, a, self); break;
      case 0: r = ((f0)f)(self); break;
      case 1: r = ((f1)f)(self, a[0]); break;
      case 2: r = ((f2)f)(self, a[0], a[1]); break;
      case 3: r = ((f3)f)(self, a[0], a[1], a[2]); break;
      case 4: r = ((f4)f)(self, a[0], a[1], a[2], a[3]); break;
      case 5: r = ((f5)f)(self, a[0], a[1], a[2], a[3], a[4]); break;
      case 6: r = ((f6)f)(self, a[0], a[1], a[2], a[3], a[4], a[5]); break;
      case 7: r = ((f7)f)(self, a[0], a[1], a[2], a[3], a[4], a[5], a[6]); break;
      case 8: r = ((f8)f)(self, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]); break;
      case 9: r = ((f9)f)(self, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8]); break;
      case 10: r = ((f10)f)(self, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9]); break;
      default: r = kn_capi_raise(rb_eNotImpError, "C methods with more than 10 fixed arguments"); break;
    }
  } else {
    r = Qundef; /* an exception is pending in the runtime */
  }
  kn_jmp_depth--;
  return r;
}

__attribute__((constructor)) static void kn_capi_init(void) { kn_capi_register_caller(kn_call); }

/* ---------- formatting (rb_sprintf: printf, plus "%"PRIsVALUE for objects) ---------- */

typedef struct { char *p; size_t len, cap; } kn_buf;
static void kn_buf_put(kn_buf *b, const char *s, size_t n) {
  if (b->len + n + 1 > b->cap) {
    size_t c = b->cap ? b->cap * 2 : 256;
    while (c < b->len + n + 1) c *= 2;
    b->p = (char *)realloc(b->p, c);
    b->cap = c;
  }
  memcpy(b->p + b->len, s, n);
  b->len += n;
  b->p[b->len] = 0;
}
/* Format into a new String (UTF-8). The buffer is freed before anything that can raise
 * runs: an object's to_s / inspect is taken first. */
static VALUE kn_vsprintf(const char *fmt, va_list ap0) {
  kn_buf b = {0, 0, 0};
  va_list ap;
  va_copy(ap, ap0);
  const char *p = fmt;
  VALUE out = rb_utf8_str_new("", 0);
  while (*p) {
    if (*p != '%') {
      const char *q = p;
      while (*q && *q != '%') q++;
      kn_buf_put(&b, p, q - p);
      p = q;
      continue;
    }
    if (p[1] == '%') { kn_buf_put(&b, "%", 1); p += 2; continue; }
    /* %[flags][width][.precision][length]conversion, '*' taken from the arguments */
    char flags[8];
    int nf = 0, plus = 0, width = -1, prec = -1;
    p++;
    while (*p && strchr("-+ #0", *p)) { if (*p == '+') plus = 1; if (nf < 7) flags[nf++] = *p; p++; }
    flags[nf] = 0;
    if (*p == '*') { width = va_arg(ap, int); p++; } else if (*p >= '0' && *p <= '9') { width = 0; while (*p >= '0' && *p <= '9') width = width * 10 + (*p++ - '0'); }
    if (*p == '.') {
      p++;
      prec = 0;
      if (*p == '*') { prec = va_arg(ap, int); p++; } else while (*p >= '0' && *p <= '9') prec = prec * 10 + (*p++ - '0');
    }
    char len[3] = {0, 0, 0};
    if (*p == 'h' || *p == 'l' || *p == 'z' || *p == 'j' || *p == 't' || *p == 'L' || *p == 'q') {
      len[0] = *p++;
      if ((len[0] == 'h' || len[0] == 'l') && *p == len[0]) len[1] = *p++;
    }
    char conv = *p ? *p++ : 0;
    if (conv == 'i' && *p == '\v') {
      /* "%"PRIsVALUE: an object, by to_s ("%+"PRIsVALUE: inspect) */
      p++;
      VALUE v = va_arg(ap, VALUE);
      if (b.len) rb_str_cat(out, b.p, b.len);
      b.len = 0;
      VALUE s = plus ? rb_inspect(v) : rb_obj_as_string(v);
      rb_str_cat(out, RSTRING_PTR(s), RSTRING_LEN(s));
      continue;
    }
    char full[64];
    size_t m = (size_t)snprintf(full, sizeof full, "%%%s", flags);
    if (width >= 0) m += snprintf(full + m, sizeof full - m, "%d", width);
    if (prec >= 0) m += snprintf(full + m, sizeof full - m, ".%d", prec);
    for (int i = 0; len[i]; i++) full[m++] = len[i];
    full[m++] = conv;
    full[m] = 0;
    char tmp[512];
    int k = 0;
    switch (conv) {
      case 'd': case 'i': case 'u': case 'o': case 'x': case 'X': case 'c':
        if (len[0] == 'l' && len[1] == 'l') k = snprintf(tmp, sizeof tmp, full, va_arg(ap, long long));
        else if (len[0] == 'l' || len[0] == 'z' || len[0] == 'j' || len[0] == 't' || len[0] == 'q') k = snprintf(tmp, sizeof tmp, full, va_arg(ap, long));
        else k = snprintf(tmp, sizeof tmp, full, va_arg(ap, int));
        break;
      case 'e': case 'E': case 'f': case 'F': case 'g': case 'G': case 'a': case 'A':
        if (len[0] == 'L') k = snprintf(tmp, sizeof tmp, full, va_arg(ap, long double));
        else k = snprintf(tmp, sizeof tmp, full, va_arg(ap, double));
        break;
      case 's': {
        const char *s = va_arg(ap, const char *);
        int need = snprintf(NULL, 0, full, s ? s : "(null)");
        char *big = (char *)malloc(need + 1);
        snprintf(big, need + 1, full, s ? s : "(null)");
        kn_buf_put(&b, big, need);
        free(big);
        continue;
      }
      case 'p': k = snprintf(tmp, sizeof tmp, full, va_arg(ap, void *)); break;
      case 'n': (void)va_arg(ap, int *); break;
      default: kn_buf_put(&b, full, m); break;
    }
    if (k > 0) kn_buf_put(&b, tmp, (size_t)k < sizeof tmp ? (size_t)k : sizeof tmp - 1);
  }
  va_end(ap);
  if (b.len) rb_str_cat(out, b.p, b.len);
  free(b.p);
  return out;
}
VALUE rb_vsprintf(const char *fmt, va_list ap) { return kn_vsprintf(fmt, ap); }
VALUE rb_sprintf(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  VALUE s = kn_vsprintf(fmt, ap);
  va_end(ap);
  return s;
}
VALUE rb_str_vcatf(VALUE s, const char *fmt, va_list ap) {
  VALUE t = kn_vsprintf(fmt, ap);
  return rb_str_cat(s, RSTRING_PTR(t), RSTRING_LEN(t));
}
VALUE rb_str_catf(VALUE s, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  rb_str_vcatf(s, fmt, ap);
  va_end(ap);
  return s;
}
int ruby_vsnprintf(char *buf, size_t n, const char *fmt, va_list ap) { return vsnprintf(buf, n, fmt, ap); }
int ruby_snprintf(char *buf, size_t n, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int r = vsnprintf(buf, n, fmt, ap);
  va_end(ap);
  return r;
}

/* ---------- calling Ruby ---------- */

VALUE rb_funcallv(VALUE recv, ID mid, int argc, const VALUE *argv) {
  VALUE r = kn_capi_funcallv(recv, mid, argc, argv);
  KN_CHECK(r);
  return r;
}
VALUE rb_funcall(VALUE recv, ID mid, int n, ...) {
  VALUE args[16];
  va_list ap;
  va_start(ap, n);
  for (int i = 0; i < n && i < 16; i++) args[i] = va_arg(ap, VALUE);
  va_end(ap);
  return rb_funcallv(recv, mid, n, args);
}
static void kn_kw_last(int argc, const VALUE *argv, int kw_splat) {
  if (kw_splat && argc > 0) kn_capi_kwhash(argv[argc - 1]);
}
VALUE rb_funcallv_kw(VALUE recv, ID mid, int argc, const VALUE *argv, int kw_splat) {
  kn_kw_last(argc, argv, kw_splat);
  return rb_funcallv(recv, mid, argc, argv);
}
VALUE rb_funcallv_public(VALUE recv, ID mid, int argc, const VALUE *argv) { return rb_funcallv(recv, mid, argc, argv); }
VALUE rb_funcallv_public_kw(VALUE recv, ID mid, int argc, const VALUE *argv, int kw_splat) { return rb_funcallv_kw(recv, mid, argc, argv, kw_splat); }
VALUE rb_funcall_with_block(VALUE recv, ID mid, int argc, const VALUE *argv, VALUE blk) {
  VALUE r = kn_capi_funcall_with_block(recv, mid, argc, argv, blk);
  KN_CHECK(r);
  return r;
}
VALUE rb_funcall_passing_block(VALUE recv, ID mid, int argc, const VALUE *argv) {
  return rb_funcall_with_block(recv, mid, argc, argv, rb_block_given_p() ? rb_block_proc() : Qnil);
}
VALUE rb_funcall_passing_block_kw(VALUE recv, ID mid, int argc, const VALUE *argv, int kw_splat) {
  kn_kw_last(argc, argv, kw_splat);
  return rb_funcall_passing_block(recv, mid, argc, argv);
}
VALUE rb_check_funcall(VALUE recv, ID mid, int argc, const VALUE *argv) {
  if (!rb_respond_to(recv, mid)) return Qundef;
  return rb_funcallv(recv, mid, argc, argv);
}
VALUE rb_apply(VALUE recv, ID mid, VALUE args) {
  return rb_funcallv(recv, mid, (int)RARRAY_LEN(args), RARRAY_CONST_PTR(args));
}
VALUE rb_yield_values2(int n, const VALUE *argv) {
  VALUE r = kn_capi_yield_values(n, argv);
  KN_CHECK(r);
  return r;
}
VALUE rb_yield(VALUE v) {
  if (v == Qundef) return rb_yield_values2(0, NULL);
  return rb_yield_values2(1, &v);
}
VALUE rb_yield_values(int n, ...) {
  VALUE args[16];
  va_list ap;
  va_start(ap, n);
  for (int i = 0; i < n && i < 16; i++) args[i] = va_arg(ap, VALUE);
  va_end(ap);
  return rb_yield_values2(n, args);
}
VALUE rb_yield_values_kw(int n, const VALUE *argv, int kw_splat) {
  kn_kw_last(n, argv, kw_splat);
  return rb_yield_values2(n, argv);
}
VALUE rb_yield_splat(VALUE args) { return rb_yield_values2((int)RARRAY_LEN(args), RARRAY_CONST_PTR(args)); }
VALUE rb_yield_block(VALUE yielded_arg, VALUE callback_arg, int argc, const VALUE *argv, VALUE blockarg) {
  (void)yielded_arg; (void)callback_arg; (void)blockarg;
  return rb_yield_values2(argc, argv);
}
void rb_need_block(void) {
  if (!rb_block_given_p()) rb_raise(rb_eLocalJumpError, "no block given");
}
VALUE rb_block_lambda(void) { return rb_block_proc(); }
int rb_keyword_given_p(void) { return 0; }
VALUE rb_proc_new(rb_block_call_func_t func, VALUE val) { return kn_capi_proc_new((const void *)func, val); }
VALUE rb_block_call(VALUE obj, ID mid, int argc, const VALUE *argv, rb_block_call_func_t bl_proc, VALUE data2) {
  VALUE blk = bl_proc ? rb_proc_new(bl_proc, data2) : Qnil;
  VALUE r = rb_funcall_with_block(obj, mid, argc, argv, blk);
  RB_GC_GUARD(blk);
  return r;
}
VALUE rb_block_call_kw(VALUE obj, ID mid, int argc, const VALUE *argv, rb_block_call_func_t bl_proc, VALUE data2, int kw_splat) {
  kn_kw_last(argc, argv, kw_splat);
  return rb_block_call(obj, mid, argc, argv, bl_proc, data2);
}
static VALUE kn_pop_block(VALUE x) { (void)x; kn_capi_pop_block(); return Qnil; }
/* (deprecated in CRuby) it_proc runs with bl_proc as the current block, which rb_each and
 * rb_funcall_passing_block pass on */
VALUE rb_iterate(VALUE (*it_proc)(VALUE), VALUE data1, rb_block_call_func_t bl_proc, VALUE data2) {
  VALUE blk = rb_proc_new(bl_proc, data2);
  kn_capi_push_block(blk);
  VALUE r = rb_ensure(it_proc, data1, kn_pop_block, Qnil);
  RB_GC_GUARD(blk);
  return r;
}
VALUE rb_each(VALUE obj) { return rb_funcall_passing_block(obj, rb_intern("each"), 0, NULL); }
VALUE rb_proc_call(VALUE proc, VALUE args) { return rb_apply(proc, rb_intern("call"), args); }
VALUE rb_proc_call_kw(VALUE proc, VALUE args, int kw_splat) {
  kn_kw_last((int)RARRAY_LEN(args), RARRAY_CONST_PTR(args), kw_splat);
  return rb_proc_call(proc, args);
}
VALUE rb_proc_call_with_block(VALUE proc, int argc, const VALUE *argv, VALUE passed_proc) {
  return rb_funcall_with_block(proc, rb_intern("call"), argc, argv, passed_proc);
}
int rb_proc_arity(VALUE proc) { return (int)NUM2INT(rb_funcall(proc, rb_intern("arity"), 0)); }
VALUE rb_proc_lambda_p(VALUE proc) { return rb_funcall(proc, rb_intern("lambda?"), 0); }
VALUE rb_obj_is_proc(VALUE v) { return rb_obj_is_kind_of(v, rb_cProc); }
VALUE rb_obj_is_method(VALUE v) { return rb_obj_is_kind_of(v, rb_cMethod); }
VALUE rb_method_call(int argc, const VALUE *argv, VALUE method) { return rb_funcallv(method, rb_intern("call"), argc, argv); }
VALUE rb_enumeratorize(VALUE obj, VALUE meth, int argc, const VALUE *argv) {
  VALUE args[17];
  args[0] = meth;
  for (int i = 0; i < argc && i < 16; i++) args[i + 1] = argv[i];
  return rb_funcallv(obj, rb_intern("enum_for"), (argc < 16 ? argc : 16) + 1, args);
}
VALUE rb_enumeratorize_with_size(VALUE obj, VALUE meth, int argc, const VALUE *argv, VALUE (*size_fn)(VALUE, VALUE, VALUE)) {
  (void)size_fn;
  return rb_enumeratorize(obj, meth, argc, argv);
}
VALUE rb_class_new_instance(int argc, const VALUE *argv, VALUE klass) {
  VALUE r = kn_capi_class_new_instance(argc, argv, klass);
  KN_CHECK(r);
  return r;
}
VALUE rb_class_new_instance_kw(int argc, const VALUE *argv, VALUE klass, int kw_splat) {
  kn_kw_last(argc, argv, kw_splat);
  return rb_class_new_instance(argc, argv, klass);
}
void rb_obj_call_init(VALUE obj, int argc, const VALUE *argv) { rb_funcall_passing_block(obj, rb_intern("initialize"), argc, argv); }
void rb_obj_call_init_kw(VALUE obj, int argc, const VALUE *argv, int kw_splat) {
  kn_kw_last(argc, argv, kw_splat);
  rb_obj_call_init(obj, argc, argv);
}
VALUE rb_obj_alloc(VALUE klass) {
  VALUE r = kn_capi_obj_alloc(klass);
  KN_CHECK(r);
  return r;
}
VALUE rb_eval_string(const char *src) { return rb_funcall(rb_cObject, rb_intern("eval"), 1, rb_utf8_str_new_cstr(src)); }
static VALUE kn_eval_string(VALUE src) { return rb_eval_string((const char *)src); }
VALUE rb_eval_string_protect(const char *src, int *state) { return rb_protect(kn_eval_string, (VALUE)src, state); }
VALUE rb_require(const char *feature) { return rb_funcall(rb_cObject, rb_intern("require"), 1, rb_utf8_str_new_cstr(feature)); }
VALUE rb_f_require(VALUE self, VALUE feature) { (void)self; return rb_funcall(rb_cObject, rb_intern("require"), 1, feature); }

/* ---------- exceptions ---------- */

VALUE rb_exc_new_str(VALUE klass, VALUE msg) { return rb_funcall(klass, rb_intern("new"), 1, msg); }
VALUE rb_exc_new(VALUE klass, const char *p, long len) { return rb_exc_new_str(klass, rb_utf8_str_new(p, len)); }
VALUE rb_exc_new_cstr(VALUE klass, const char *p) { return rb_exc_new_str(klass, rb_utf8_str_new_cstr(p)); }
void rb_exc_raise(VALUE e) {
  kn_capi_exc_raise(e);
  kn_unwind();
}
void rb_raise(VALUE klass, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  VALUE msg = kn_vsprintf(fmt, ap);
  va_end(ap);
  rb_exc_raise(rb_exc_new_str(klass, msg));
}
void rb_enc_raise(rb_encoding *enc, VALUE klass, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  VALUE msg = kn_vsprintf(fmt, ap);
  va_end(ap);
  if (enc) rb_enc_associate(msg, enc);
  rb_exc_raise(rb_exc_new_str(klass, msg));
}
void rb_name_error(ID name, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  VALUE msg = kn_vsprintf(fmt, ap);
  va_end(ap);
  VALUE args[2] = {msg, ID2SYM(name)};
  rb_exc_raise(rb_class_new_instance(2, args, rb_eNameError));
}
void rb_frozen_error_raise(VALUE obj, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  VALUE msg = kn_vsprintf(fmt, ap);
  va_end(ap);
  (void)obj;
  rb_exc_raise(rb_exc_new_str(rb_eFrozenError, msg));
}
void rb_error_frozen(const char *what) { rb_raise(rb_eFrozenError, "can't modify frozen %s", what); }
void rb_error_frozen_object(VALUE obj) { rb_check_frozen(obj); rb_raise(rb_eFrozenError, "can't modify frozen object"); }
void rb_check_frozen(VALUE v) { KN_CHECK(kn_capi_check_frozen(v)); }
void rb_error_arity(int argc, int min, int max) {
  if (min == max) rb_raise(rb_eArgError, "wrong number of arguments (given %d, expected %d)", argc, min);
  else if (max == UNLIMITED_ARGUMENTS) rb_raise(rb_eArgError, "wrong number of arguments (given %d, expected %d+)", argc, min);
  else rb_raise(rb_eArgError, "wrong number of arguments (given %d, expected %d..%d)", argc, min, max);
}
void rb_invalid_str(const char *str, const char *type) {
  VALUE s = rb_utf8_str_new_cstr(str);
  rb_raise(rb_eArgError, "invalid value for %s: %+" PRIsVALUE, type, s);
}
void rb_memerror(void) {
  fprintf(stderr, "[FATAL] failed to allocate memory\n");
  abort();
}
void rb_notimplement(void) { rb_raise(rb_eNotImpError, "function is unimplemented on this machine"); }
void rb_bug(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  fprintf(stderr, "[BUG] ");
  vfprintf(stderr, fmt, ap);
  fprintf(stderr, "\n");
  va_end(ap);
  abort();
}
void rb_fatal(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  fprintf(stderr, "[FATAL] ");
  vfprintf(stderr, fmt, ap);
  fprintf(stderr, "\n");
  va_end(ap);
  exit(1);
}
VALUE rb_syserr_new(int err, const char *msg) {
  VALUE args[2] = {msg ? rb_utf8_str_new_cstr(msg) : Qnil, INT2FIX(err)};
  return rb_class_new_instance(2, args, rb_eSystemCallError);
}
void rb_syserr_fail(int err, const char *msg) { rb_exc_raise(rb_syserr_new(err, msg)); }
void rb_syserr_fail_str(int err, VALUE msg) {
  VALUE args[2] = {msg, INT2FIX(err)};
  rb_exc_raise(rb_class_new_instance(2, args, rb_eSystemCallError));
}
void rb_sys_fail(const char *msg) { rb_syserr_fail(errno, msg); }
void rb_sys_fail_str(VALUE msg) { rb_syserr_fail_str(errno, msg); }
void rb_throw_obj(VALUE tag, VALUE val) {
  rb_funcall(rb_cObject, rb_intern("throw"), 2, tag, val);
  kn_unwind();
}
void rb_throw(const char *tag, VALUE val) { rb_throw_obj(ID2SYM(rb_intern(tag)), val); }
void rb_exit(int status) {
  rb_funcall(rb_cObject, rb_intern("exit"), 1, INT2FIX(status));
  kn_unwind();
}
static void kn_warn_v(const char *fmt, va_list ap) {
  VALUE msg = kn_vsprintf(fmt, ap);
  kn_capi_warn(StringValueCStr(msg));
}
void rb_warn(const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  kn_warn_v(fmt, ap);
  va_end(ap);
}
void rb_category_warn(int category, const char *fmt, ...) {
  (void)category;
  va_list ap;
  va_start(ap, fmt);
  kn_warn_v(fmt, ap);
  va_end(ap);
}
/* (only with $VERBOSE = true) */
void rb_warning(const char *fmt, ...) {
  if (rb_gv_get("$VERBOSE") != Qtrue) return;
  va_list ap;
  va_start(ap, fmt);
  kn_warn_v(fmt, ap);
  va_end(ap);
}
static __thread VALUE kn_verbose, kn_debug;
VALUE *rb_ruby_verbose_ptr(void) { kn_verbose = rb_gv_get("$VERBOSE"); return &kn_verbose; }
VALUE *rb_ruby_debug_ptr(void) { kn_debug = rb_gv_get("$DEBUG"); return &kn_debug; }

/* rb_protect / rb_jump_tag: an exception is state TAG_RAISE (6), with the exception in
 * rb_errinfo(); a throw / break unwinding through is TAG_THROW (7), resumed by rb_jump_tag. */
#define KN_TAG_RAISE 6
#define KN_TAG_THROW 7
static __thread VALUE kn_saved_throw = Qnil;
/* (exc: the exception, now also $!; prev: $! before it) */
static int kn_take_state(VALUE *exc, VALUE *prev) {
  if (prev) *prev = rb_errinfo();
  VALUE e = kn_capi_take_pending();
  if (e != Qnil) { if (exc) *exc = e; return KN_TAG_RAISE; }
  kn_saved_throw = kn_capi_save_throw();
  if (exc) *exc = Qnil;
  return KN_TAG_THROW;
}
void rb_jump_tag(int state) {
  if (state == KN_TAG_RAISE) rb_exc_raise(rb_errinfo());
  if (state == KN_TAG_THROW) kn_capi_restore_throw(kn_saved_throw);
  kn_unwind();
}
VALUE rb_protect(VALUE (*func)(VALUE), VALUE arg, int *state) {
  jmp_buf jb;
  volatile VALUE r = Qnil;
  kn_jmp_stack[kn_jmp_depth++] = &jb;
  if (setjmp(jb) == 0) {
    r = func(arg);
    kn_jmp_depth--;
    if (state) *state = 0;
    return r;
  }
  kn_jmp_depth--;
  int st = kn_take_state(NULL, NULL);
  if (state) *state = st;
  return Qnil;
}
VALUE rb_ensure(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*e_proc)(VALUE), VALUE data2) {
  jmp_buf jb;
  volatile VALUE r = Qnil;
  kn_jmp_stack[kn_jmp_depth++] = &jb;
  if (setjmp(jb) == 0) {
    r = b_proc(data1);
    kn_jmp_depth--;
    e_proc(data2);
    return r;
  }
  kn_jmp_depth--;
  VALUE e = Qnil;
  VALUE prev = Qnil;
  int st = kn_take_state(&e, &prev);
  VALUE saved = kn_saved_throw;
  e_proc(data2);
  if (st == KN_TAG_RAISE) {
    rb_set_errinfo(prev);
    rb_exc_raise(e);
  }
  kn_capi_restore_throw(saved);
  kn_unwind();
}
#define KN_MAX_RESCUE 8
static VALUE kn_rescue(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2, const VALUE *classes, int n) {
  jmp_buf jb;
  volatile VALUE r = Qnil;
  kn_jmp_stack[kn_jmp_depth++] = &jb;
  if (setjmp(jb) == 0) {
    r = b_proc(data1);
    kn_jmp_depth--;
    return r;
  }
  kn_jmp_depth--;
  VALUE e = Qnil;
  VALUE prev = Qnil;
  int st = kn_take_state(&e, &prev);
  if (st != KN_TAG_RAISE) rb_jump_tag(st);
  for (int i = 0; i < n; i++) {
    if (RTEST(rb_obj_is_kind_of(e, classes[i]))) {
      VALUE res = r_proc ? r_proc(data2, e) : Qnil;
      rb_set_errinfo(prev);
      return res;
    }
  }
  rb_set_errinfo(prev);
  rb_exc_raise(e);
}
VALUE rb_rescue(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2) {
  VALUE c = rb_eStandardError;
  return kn_rescue(b_proc, data1, r_proc, data2, &c, 1);
}
VALUE rb_rescue2(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2, ...) {
  VALUE classes[KN_MAX_RESCUE];
  int n = 0;
  va_list ap;
  va_start(ap, data2);
  for (VALUE k; n < KN_MAX_RESCUE && (k = va_arg(ap, VALUE)) != 0;) classes[n++] = k;
  va_end(ap);
  if (n == 0) classes[n++] = rb_eStandardError;
  return kn_rescue(b_proc, data1, r_proc, data2, classes, n);
}

/* ---------- type checks and conversions ---------- */

void rb_check_type(VALUE v, int t) { KN_CHECK(kn_capi_check_type(v, t)); }
long rb_num2long(VALUE v) {
  if (FIXNUM_P(v)) return FIX2LONG(v);
  int ok;
  long r = kn_capi_num2long(v, &ok);
  if (!ok) kn_unwind();
  return r;
}
unsigned long rb_num2ulong(VALUE v) {
  int ok;
  unsigned long r = kn_capi_num2ulong(v, &ok);
  if (!ok) kn_unwind();
  return r;
}
long long rb_num2ll(VALUE v) { return rb_num2long(v); }
unsigned long long rb_num2ull(VALUE v) { return rb_num2ulong(v); }
long rb_num2int(VALUE v) {
  long l = rb_num2long(v);
  if (l > INT_MAX || l < INT_MIN) rb_raise(rb_eRangeError, "integer %ld too %s to convert to 'int'", l, l < 0 ? "small" : "big");
  return l;
}
long rb_fix2int(VALUE v) { return rb_num2int(v); }
unsigned long rb_num2uint(VALUE v) {
  long l = rb_num2long(v);
  if (l > (long)UINT_MAX || l < INT_MIN) rb_raise(rb_eRangeError, "integer %ld too %s to convert to 'unsigned int'", l, l < 0 ? "small" : "big");
  return (unsigned long)(unsigned int)l;
}
short rb_num2short(VALUE v) {
  long l = rb_num2long(v);
  if (l > SHRT_MAX || l < SHRT_MIN) rb_raise(rb_eRangeError, "integer %ld too %s to convert to 'short'", l, l < 0 ? "small" : "big");
  return (short)l;
}
unsigned short rb_num2ushort(VALUE v) {
  long l = rb_num2long(v);
  if (l > USHRT_MAX || l < SHRT_MIN) rb_raise(rb_eRangeError, "integer %ld too %s to convert to 'unsigned short'", l, l < 0 ? "small" : "big");
  return (unsigned short)l;
}
char rb_num2chr(VALUE v) {
  if (RB_TYPE_P(v, T_STRING) && RSTRING_LEN(v) >= 1) return RSTRING_PTR(v)[0];
  return (char)(rb_num2int(v) & 0xff);
}
double rb_num2dbl(VALUE v) {
  int ok;
  double r = kn_capi_num2dbl(v, &ok);
  if (!ok) kn_unwind();
  return r;
}
unsigned long long rb_big2ull(VALUE v) {
  int ok;
  unsigned long long r = kn_capi_big2ull(v, &ok);
  if (!ok) kn_unwind();
  return r;
}
long long rb_big2ll(VALUE v) {
  int ok;
  long long r = kn_capi_big2ll(v, &ok);
  if (!ok) kn_unwind();
  return r;
}
long rb_big2long(VALUE v) { return (long)rb_big2ll(v); }
unsigned long rb_big2ulong(VALUE v) { return (unsigned long)rb_big2ull(v); }
VALUE rb_int2big(intptr_t n) { return rb_int2inum(n); }
VALUE rb_uint2big(uintptr_t n) { return rb_uint2inum(n); }
VALUE rb_big2str(VALUE b, int base) { return rb_funcall(b, rb_intern("to_s"), 1, INT2FIX(base)); }
VALUE rb_big_cmp(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("<=>"), 1, b); }
VALUE rb_big_eq(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("=="), 1, b); }
VALUE rb_big_plus(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("+"), 1, b); }
VALUE rb_big_minus(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("-"), 1, b); }
VALUE rb_big_mul(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("*"), 1, b); }
VALUE rb_big_norm(VALUE b) { return b; }
VALUE rb_dbl2big(double d) { return rb_funcall(rb_float_new(d), rb_intern("to_i"), 0); }
VALUE rb_float_new_in_heap(double d) { return rb_float_new(d); }
VALUE rb_Integer(VALUE v) { return rb_funcall(rb_cObject, rb_intern("Integer"), 1, v); }
VALUE rb_Float(VALUE v) { return rb_funcall(rb_cObject, rb_intern("Float"), 1, v); }
VALUE rb_String(VALUE v) { return rb_funcall(rb_cObject, rb_intern("String"), 1, v); }
VALUE rb_Array(VALUE v) { return rb_funcall(rb_cObject, rb_intern("Array"), 1, v); }
VALUE rb_Hash(VALUE v) { return rb_funcall(rb_cObject, rb_intern("Hash"), 1, v); }
VALUE rb_Rational(VALUE num, VALUE den) { return rb_funcall(rb_cObject, rb_intern("Rational"), 2, num, den); }
VALUE rb_rational_new(VALUE num, VALUE den) { return rb_Rational(num, den); }
VALUE rb_complex_new(VALUE re, VALUE im) { return rb_funcall(rb_cObject, rb_intern("Complex"), 2, re, im); }
static const char *kn_type_name(VALUE v) {
  if (v == Qnil) return "nil";
  if (v == Qtrue) return "true";
  if (v == Qfalse) return "false";
  return rb_obj_classname(v);
}
static VALUE kn_convert(VALUE v, int type, const char *tname, const char *method, int raise) {
  if (type == T_FIXNUM ? RB_INTEGER_TYPE_P(v) : rb_type(v) == type) return v;
  ID m = rb_intern(method);
  if (!rb_respond_to(v, m)) {
    if (!raise) return Qnil;
    int implicit = !strcmp(method, "to_str") || !strcmp(method, "to_ary") || !strcmp(method, "to_hash") || !strcmp(method, "to_int") || !strcmp(method, "to_sym");
    if (implicit) rb_raise(rb_eTypeError, "no implicit conversion of %s into %s", kn_type_name(v), tname);
    rb_raise(rb_eTypeError, "can't convert %s into %s", kn_type_name(v), tname);
  }
  VALUE r = rb_funcall(v, m, 0);
  if (!raise && r == Qnil) return Qnil;
  if (type == T_FIXNUM ? !RB_INTEGER_TYPE_P(r) : rb_type(r) != type)
    rb_raise(rb_eTypeError, "can't convert %s to %s (%s#%s gives %s)", kn_type_name(v), tname, kn_type_name(v), method, kn_type_name(r));
  return r;
}
VALUE rb_convert_type(VALUE v, int type, const char *tname, const char *method) { return kn_convert(v, type, tname, method, 1); }
VALUE rb_check_convert_type(VALUE v, int type, const char *tname, const char *method) { return kn_convert(v, type, tname, method, 0); }
VALUE rb_check_string_type(VALUE v) { return kn_convert(v, T_STRING, "String", "to_str", 0); }
VALUE rb_check_array_type(VALUE v) { return kn_convert(v, T_ARRAY, "Array", "to_ary", 0); }
VALUE rb_check_hash_type(VALUE v) { return kn_convert(v, T_HASH, "Hash", "to_hash", 0); }
VALUE rb_str_to_str(VALUE v) { return kn_convert(v, T_STRING, "String", "to_str", 1); }
VALUE rb_to_int(VALUE v) { return kn_convert(v, T_FIXNUM, "Integer", "to_int", 1); }
VALUE rb_check_to_integer(VALUE v, const char *method) {
  if (RB_INTEGER_TYPE_P(v)) return v;
  ID m = rb_intern(method);
  if (!rb_respond_to(v, m)) return Qnil;
  VALUE r = rb_funcall(v, m, 0);
  return RB_INTEGER_TYPE_P(r) ? r : Qnil;
}
VALUE rb_check_to_int(VALUE v) { return rb_check_to_integer(v, "to_int"); }
VALUE rb_to_float(VALUE v) {
  if (RB_FLOAT_TYPE_P(v)) return v;
  if (!RTEST(rb_obj_is_kind_of(v, rb_cNumeric))) rb_raise(rb_eTypeError, "can't convert %s into Float", kn_type_name(v));
  return rb_convert_type(v, T_FLOAT, "Float", "to_f");
}
VALUE rb_str2inum(VALUE s, int base) { return rb_funcall(s, rb_intern("to_i"), 1, INT2FIX(base)); }
VALUE rb_cstr_to_inum(const char *s, int base, int badcheck) {
  VALUE str = rb_utf8_str_new_cstr(s);
  if (badcheck) return base ? rb_funcall(rb_cObject, rb_intern("Integer"), 2, str, INT2FIX(base)) : rb_Integer(str);
  return rb_str2inum(str, base);
}
VALUE rb_cstr2inum(const char *s, int base) { return rb_cstr_to_inum(s, base, base == 0); }
double rb_cstr_to_dbl(const char *s, int badcheck) {
  VALUE str = rb_utf8_str_new_cstr(s);
  return NUM2DBL(badcheck ? rb_Float(str) : rb_funcall(str, rb_intern("to_f"), 0));
}
double rb_str_to_dbl(VALUE s, int badcheck) { return NUM2DBL(badcheck ? rb_Float(s) : rb_funcall(s, rb_intern("to_f"), 0)); }

/* ---------- strings ---------- */

VALUE rb_string_value(volatile VALUE *p) {
  VALUE r = kn_capi_string_value(*p);
  KN_CHECK(r);
  *p = r;
  return r;
}
char *rb_string_value_ptr(volatile VALUE *p) { return RSTRING_PTR(rb_string_value(p)); }
char *rb_string_value_cstr(volatile VALUE *p) {
  VALUE s = rb_string_value(p);
  char *c = RSTRING_PTR(s);
  if ((long)strlen(c) != RSTRING_LEN(s)) rb_raise(rb_eArgError, "string contains null byte");
  return c;
}
VALUE rb_str_cat(VALUE s, const char *p, long len) {
  rb_check_frozen(s);
  return kn_capi_str_cat(s, p, len);
}
VALUE rb_str_cat_cstr(VALUE s, const char *p) { return rb_str_cat(s, p, (long)strlen(p)); }
VALUE rb_enc_str_buf_cat(VALUE s, const char *p, long len, rb_encoding *enc) {
  int ascii = rb_enc_str_asciionly_p(s);
  rb_str_cat(s, p, len);
  if (enc && ascii && rb_enc_get_index(s) != rb_enc_to_index(enc)) {
    long i;
    for (i = 0; i < len && (unsigned char)p[i] < 128; i++) {}
    if (i < len) rb_enc_associate(s, enc);
  }
  return s;
}
VALUE rb_str_buf_new_cstr(const char *p) {
  VALUE s = rb_str_buf_new((long)strlen(p));
  return rb_str_cat_cstr(s, p);
}
void rb_str_modify(VALUE s) { rb_check_frozen(s); }
void rb_str_modify_expand(VALUE s, long extra) {
  rb_check_frozen(s);
  if (extra < 0) rb_raise(rb_eArgError, "negative expanding string size");
  kn_capi_str_reserve(s, extra);
}
VALUE rb_str_freeze(VALUE s) { return rb_obj_freeze(s); }
VALUE rb_str_resurrect(VALUE s) { return rb_str_dup(s); }
VALUE rb_str_new_shared(VALUE s) { return rb_str_dup(s); }
VALUE rb_str_locktmp(VALUE s) { return s; }
VALUE rb_str_unlocktmp(VALUE s) { return s; }
void rb_str_free(VALUE s) { (void)s; }
VALUE rb_str_buf_append(VALUE s, VALUE o) { return rb_funcall(s, rb_intern("<<"), 1, o); }
VALUE rb_str_concat(VALUE s, VALUE o) { return rb_funcall(s, rb_intern("concat"), 1, o); }
VALUE rb_str_plus(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("+"), 1, b); }
VALUE rb_str_times(VALUE s, VALUE n) { return rb_funcall(s, rb_intern("*"), 1, n); }
VALUE rb_str_length(VALUE s) { return rb_funcall(s, rb_intern("length"), 0); }
long rb_str_strlen(VALUE s) { return NUM2LONG(rb_str_length(s)); }
VALUE rb_str_inspect(VALUE s) { return rb_inspect(s); }
VALUE rb_str_dump(VALUE s) { return rb_funcall(s, rb_intern("dump"), 0); }
VALUE rb_str_split(VALUE s, const char *sep) { return rb_funcall(s, rb_intern("split"), 1, rb_utf8_str_new_cstr(sep)); }
VALUE rb_str_succ(VALUE s) { return rb_funcall(s, rb_intern("succ"), 0); }
VALUE rb_str_replace(VALUE s, VALUE o) { return rb_funcall(s, rb_intern("replace"), 1, o); }
VALUE rb_str_substr(VALUE s, long beg, long len) { return rb_funcall(s, rb_intern("[]"), 2, LONG2NUM(beg), LONG2NUM(len)); }
VALUE rb_str_scrub(VALUE s, VALUE repl) {
  if (RTEST(rb_funcall(s, rb_intern("valid_encoding?"), 0))) return Qnil;
  return NIL_P(repl) ? rb_funcall(s, rb_intern("scrub"), 0) : rb_funcall(s, rb_intern("scrub"), 1, repl);
}
VALUE rb_obj_as_string(VALUE v);
VALUE rb_inspect(VALUE v) {
  VALUE r = kn_capi_inspect(v);
  KN_CHECK(r);
  return r;
}
VALUE rb_any_to_s(VALUE v) { return rb_sprintf("#<%s:%p>", rb_obj_classname(v), (void *)v); }
VALUE rb_str_encode(VALUE str, VALUE to, int ecflags, VALUE ecopts) {
  VALUE opts = NIL_P(ecopts) ? rb_hash_new() : rb_hash_dup(ecopts);
  if (ecflags & ECONV_INVALID_REPLACE) rb_hash_aset(opts, ID2SYM(rb_intern("invalid")), ID2SYM(rb_intern("replace")));
  if (ecflags & ECONV_UNDEF_REPLACE) rb_hash_aset(opts, ID2SYM(rb_intern("undef")), ID2SYM(rb_intern("replace")));
  if (ecflags & ECONV_XML_TEXT_DECORATOR) rb_hash_aset(opts, ID2SYM(rb_intern("xml")), ID2SYM(rb_intern("text")));
  if (RHASH_SIZE(opts) == 0) return rb_funcall(str, rb_intern("encode"), 1, to);
  VALUE args[2] = {to, kn_capi_kwhash(opts)};
  return rb_funcallv(str, rb_intern("encode"), 2, args);
}
VALUE rb_str_export_to_enc(VALUE str, rb_encoding *enc) { return rb_str_encode(str, rb_enc_from_encoding(enc), 0, Qnil); }
static VALUE kn_conv_enc(VALUE args) {
  VALUE *a = (VALUE *)args;
  return rb_str_encode(a[0], a[1], 0, Qnil);
}
VALUE rb_str_conv_enc(VALUE str, rb_encoding *from, rb_encoding *to) {
  (void)from;
  if (!to || rb_enc_get_index(str) == rb_enc_to_index(to)) return str;
  VALUE args[2] = {str, rb_enc_from_encoding(to)};
  int state = 0;
  VALUE r = rb_protect(kn_conv_enc, (VALUE)args, &state);
  if (state) { rb_set_errinfo(Qnil); return str; }
  return r;
}
rb_encoding *rb_to_encoding(VALUE enc) {
  int i = rb_to_encoding_index(enc);
  if (i < 0) {
    if (RB_TYPE_P(enc, T_STRING)) rb_raise(rb_eArgError, "unknown encoding name - %" PRIsVALUE, enc);
    rb_raise(rb_eTypeError, "wrong argument type %s (expected Encoding)", kn_type_name(enc));
  }
  return rb_enc_from_index(i);
}
VALUE rb_enc_default_external(void) { return rb_funcall(rb_cEncoding, rb_intern("default_external"), 0); }
VALUE rb_enc_default_internal(void) { return rb_funcall(rb_cEncoding, rb_intern("default_internal"), 0); }
rb_encoding *rb_default_external_encoding(void) { return rb_to_encoding(rb_enc_default_external()); }
rb_encoding *rb_default_internal_encoding(void) {
  VALUE e = rb_enc_default_internal();
  return NIL_P(e) ? NULL : rb_to_encoding(e);
}
rb_encoding *rb_enc_check(VALUE a, VALUE b) {
  rb_encoding *e = rb_enc_compatible(a, b);
  if (!e) rb_raise(rb_eEncCompatError, "incompatible character encodings: %s and %s", rb_enc_name(rb_enc_get(a)), rb_enc_name(rb_enc_get(b)));
  return e;
}
/* characters: UTF-8 is decoded; other encodings go by their min/max character length */
static int kn_utf8_len(const unsigned char *p, const unsigned char *e) {
  if (p >= e) return -2;
  unsigned c = p[0];
  int n = c < 0x80 ? 1 : (c & 0xe0) == 0xc0 ? 2 : (c & 0xf0) == 0xe0 ? 3 : (c & 0xf8) == 0xf0 ? 4 : 0;
  if (n == 0 || (n == 2 && c < 0xc2) || (n == 4 && c > 0xf4)) return -1;
  for (int i = 1; i < n; i++) {
    if (p + i >= e) return -1 - (n - i);
    if ((p[i] & 0xc0) != 0x80) return -1;
  }
  if (n == 3 && ((c == 0xe0 && p[1] < 0xa0) || (c == 0xed && p[1] > 0x9f))) return -1;
  if (n == 4 && ((c == 0xf0 && p[1] < 0x90) || (c == 0xf4 && p[1] > 0x8f))) return -1;
  return n;
}
int rb_enc_precise_mbclen(const char *p, const char *e, rb_encoding *enc) {
  if (!enc || enc->index == 1 || !strcmp(enc->name, "UTF8-MAC") || !strcmp(enc->name, "CESU-8")) return kn_utf8_len((const unsigned char *)p, (const unsigned char *)e);
  if (p >= e) return -2;
  int n = enc->min_len;
  if (enc->max_len > enc->min_len && enc->ascii_compat && (unsigned char)*p >= 0x80) n = 2;
  if (e - p < n) return -1 - (int)(n - (e - p));
  return n;
}
int rb_enc_mbclen(const char *p, const char *e, rb_encoding *enc) {
  int n = rb_enc_precise_mbclen(p, e, enc);
  if (n > 0) return n;
  int m = enc ? enc->min_len : 1;
  return (e - p) < m ? (int)(e - p) : m;
}
long rb_enc_strlen(const char *p, const char *e, rb_encoding *enc) {
  long n = 0;
  while (p < e) { p += rb_enc_mbclen(p, e, enc); n++; }
  return n;
}
int rb_enc_codelen(int code, rb_encoding *enc) {
  if (!enc || enc->index == 1) return code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  return enc->min_len;
}
unsigned int rb_enc_codepoint_len(const char *p, const char *e, int *len, rb_encoding *enc) {
  if (p >= e) rb_raise(rb_eArgError, "empty string");
  int n = rb_enc_precise_mbclen(p, e, enc);
  if (n <= 0) rb_raise(rb_eArgError, "invalid byte sequence in %s", enc ? enc->name : "UTF-8");
  if (len) *len = n;
  const unsigned char *u = (const unsigned char *)p;
  if (!enc || enc->index == 1) {
    if (n == 1) return u[0];
    unsigned c = u[0] & (0x7f >> n);
    for (int i = 1; i < n; i++) c = (c << 6) | (u[i] & 0x3f);
    return c;
  }
  unsigned c = 0;
  for (int i = 0; i < n; i++) c = (c << 8) | u[i];
  return c;
}
int rb_enc_uint_chr(unsigned int code, rb_encoding *enc) {
  return (int)rb_funcall(UINT2NUM(code), rb_intern("chr"), 1, rb_enc_from_encoding(enc));
}

/* ---------- symbols ---------- */

VALUE rb_to_symbol(VALUE name) {
  if (SYMBOL_P(name)) return name;
  return rb_str_intern(rb_str_to_str(name));
}
ID rb_check_id(volatile VALUE *namep) {
  VALUE n = *namep;
  if (SYMBOL_P(n)) return SYM2ID(n);
  VALUE s = rb_check_string_type(n);
  if (NIL_P(s)) rb_raise(rb_eTypeError, "%+" PRIsVALUE " is not a symbol nor a string", n);
  *namep = s;
  return rb_intern_str(s);
}
VALUE rb_check_symbol(volatile VALUE *namep) { return ID2SYM(rb_check_id(namep)); }
int rb_is_const_id(ID id) { const char *n = rb_id2name(id); return n && n[0] >= 'A' && n[0] <= 'Z'; }
int rb_is_instance_id(ID id) { const char *n = rb_id2name(id); return n && n[0] == '@' && n[1] != '@'; }

/* ---------- arrays ---------- */

VALUE rb_ary_new_from_args(long n, ...) {
  VALUE a = rb_ary_new_capa(n);
  va_list ap;
  va_start(ap, n);
  for (long i = 0; i < n; i++) rb_ary_push(a, va_arg(ap, VALUE));
  va_end(ap);
  return a;
}
VALUE rb_assoc_new(VALUE a, VALUE b) { return rb_ary_new_from_args(2, a, b); }
VALUE rb_ary_cat(VALUE a, const VALUE *p, long n) {
  for (long i = 0; i < n; i++) rb_ary_push(a, p[i]);
  return a;
}
VALUE rb_ary_aref1(VALUE a, VALUE i) { return rb_funcall(a, rb_intern("[]"), 1, i); }
VALUE rb_ary_concat(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("concat"), 1, b); }
VALUE rb_ary_plus(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("+"), 1, b); }
VALUE rb_ary_includes(VALUE a, VALUE v) { return rb_funcall(a, rb_intern("include?"), 1, v); }
VALUE rb_ary_delete(VALUE a, VALUE v) { return rb_funcall(a, rb_intern("delete"), 1, v); }
VALUE rb_ary_delete_at(VALUE a, long i) { return rb_funcall(a, rb_intern("delete_at"), 1, LONG2NUM(i)); }
VALUE rb_ary_reverse(VALUE a) { return rb_funcall(a, rb_intern("reverse!"), 0); }
VALUE rb_ary_sort(VALUE a) { return rb_funcall(a, rb_intern("sort"), 0); }
VALUE rb_ary_sort_bang(VALUE a) { return rb_funcall(a, rb_intern("sort!"), 0); }
VALUE rb_ary_subseq(VALUE a, long beg, long len) { return rb_funcall(a, rb_intern("[]"), 2, LONG2NUM(beg), LONG2NUM(len)); }
VALUE rb_ary_freeze(VALUE a) { return rb_obj_freeze(a); }
VALUE rb_ary_resize(VALUE a, long len) {
  long n = RARRAY_LEN(a);
  if (len < n) rb_funcall(a, rb_intern("slice!"), 2, LONG2NUM(len), LONG2NUM(n - len));
  else if (len > n) rb_ary_store(a, len - 1, Qnil);
  return a;
}
VALUE rb_ary_to_ary(VALUE v) {
  VALUE a = rb_check_array_type(v);
  return NIL_P(a) ? rb_ary_new_from_args(1, v) : a;
}
VALUE rb_ary_to_s(VALUE a) { return rb_inspect(a); }
VALUE rb_ary_replace(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("replace"), 1, b); }
VALUE rb_ary_assoc(VALUE a, VALUE k) { return rb_funcall(a, rb_intern("assoc"), 1, k); }
VALUE rb_ary_cmp(VALUE a, VALUE b) { return rb_funcall(a, rb_intern("<=>"), 1, b); }

/* ---------- hashes ---------- */

VALUE rb_hash_aref(VALUE h, VALUE k) {
  VALUE r = kn_capi_hash_aref(h, k);
  KN_CHECK(r);
  return r;
}
/* The callback sees each pair the Hash had when iteration started (ST_STOP ends it,
 * ST_DELETE removes the pair). */
void rb_hash_foreach(VALUE h, int (*func)(VALUE, VALUE, VALUE), VALUE arg) {
  VALUE pairs = kn_capi_hash_pairs(h);
  long n = RARRAY_LEN(pairs);
  for (long i = 0; i + 1 < n; i += 2) {
    VALUE k = rb_ary_entry(pairs, i), v = rb_ary_entry(pairs, i + 1);
    int r = func(k, v, arg);
    if (r == ST_STOP) break;
    if (r == ST_DELETE) rb_hash_delete(h, k);
  }
  RB_GC_GUARD(pairs);
}
VALUE rb_hash_dup(VALUE h) { return rb_funcall(h, rb_intern("dup"), 0); }
VALUE rb_hash_freeze(VALUE h) { return rb_obj_freeze(h); }
VALUE rb_hash_clear(VALUE h) { return rb_funcall(h, rb_intern("clear"), 0); }
VALUE rb_hash_fetch(VALUE h, VALUE k) { return rb_funcall(h, rb_intern("fetch"), 1, k); }
VALUE rb_hash_delete_if(VALUE h) { return rb_funcall_passing_block(h, rb_intern("delete_if"), 0, NULL); }
VALUE rb_hash(VALUE obj) { return rb_funcall(obj, rb_intern("hash"), 0); }
void rb_hash_bulk_insert(long n, const VALUE *argv, VALUE h) {
  for (long i = 0; i + 1 < n; i += 2) rb_hash_aset(h, argv[i], argv[i + 1]);
}

/* ---------- structs and ranges ---------- */

static VALUE kn_struct_new_class(const char *name, va_list ap) {
  VALUE args[64];
  int n = 0;
  if (name) args[n++] = rb_utf8_str_new_cstr(name);
  for (const char *m; n < 63 && (m = va_arg(ap, const char *)) != NULL;) args[n++] = ID2SYM(rb_intern(m));
  return rb_funcallv(rb_cStruct, rb_intern("new"), n, args);
}
VALUE rb_struct_define(const char *name, ...) {
  va_list ap;
  va_start(ap, name);
  VALUE k = kn_struct_new_class(name, ap);
  va_end(ap);
  return k;
}
VALUE rb_struct_define_under(VALUE outer, const char *name, ...) {
  va_list ap;
  va_start(ap, name);
  VALUE k = kn_struct_new_class(NULL, ap);
  va_end(ap);
  rb_define_const(outer, name, k);
  return k;
}
VALUE rb_struct_s_members(VALUE klass) { return rb_funcall(klass, rb_intern("members"), 0); }
VALUE rb_struct_members(VALUE st) { return rb_funcall(st, rb_intern("members"), 0); }
VALUE rb_struct_new(VALUE klass, ...) {
  long n = RARRAY_LEN(rb_struct_s_members(klass));
  VALUE args[64];
  va_list ap;
  va_start(ap, klass);
  for (long i = 0; i < n && i < 64; i++) args[i] = va_arg(ap, VALUE);
  va_end(ap);
  return rb_class_new_instance((int)(n < 64 ? n : 64), args, klass);
}
VALUE rb_struct_alloc(VALUE klass, VALUE values) {
  return rb_class_new_instance((int)RARRAY_LEN(values), RARRAY_CONST_PTR(values), klass);
}
VALUE rb_struct_aref(VALUE st, VALUE idx) { return rb_funcall(st, rb_intern("[]"), 1, idx); }
VALUE rb_struct_aset(VALUE st, VALUE idx, VALUE v) { return rb_funcall(st, rb_intern("[]="), 2, idx, v); }
VALUE rb_struct_getmember(VALUE st, ID id) { return rb_funcall(st, rb_intern("[]"), 1, ID2SYM(id)); }
VALUE rb_struct_size(VALUE st) { return rb_funcall(st, rb_intern("size"), 0); }
VALUE rb_range_new(VALUE beg, VALUE end, int excl) { return rb_funcall(rb_cRange, rb_intern("new"), 3, beg, end, excl ? Qtrue : Qfalse); }
int rb_range_values(VALUE range, VALUE *begp, VALUE *endp, int *exclp) {
  if (!RTEST(rb_obj_is_kind_of(range, rb_cRange))) {
    ID b = rb_intern("begin"), e = rb_intern("end"), x = rb_intern("exclude_end?");
    if (!rb_respond_to(range, b) || !rb_respond_to(range, e) || !rb_respond_to(range, x)) return 0;
  }
  *begp = rb_funcall(range, rb_intern("begin"), 0);
  *endp = rb_funcall(range, rb_intern("end"), 0);
  *exclp = RTEST(rb_funcall(range, rb_intern("exclude_end?"), 0));
  return 1;
}

/* ---------- classes, constants, variables ---------- */

VALUE rb_const_get(VALUE klass, ID id) {
  VALUE r = kn_capi_const_get(klass, id);
  KN_CHECK(r);
  return r;
}
VALUE rb_const_get_at(VALUE klass, ID id) { return rb_funcall(klass, rb_intern("const_get"), 2, ID2SYM(id), Qfalse); }
VALUE rb_const_get_from(VALUE klass, ID id) { return rb_const_get(klass, id); }
int rb_const_defined_at(VALUE klass, ID id) { return RTEST(rb_funcall(klass, rb_intern("const_defined?"), 2, ID2SYM(id), Qfalse)); }
int rb_const_defined_from(VALUE klass, ID id) { return rb_const_defined(klass, id); }
VALUE rb_const_remove(VALUE klass, ID id) { return rb_funcall(klass, rb_intern("remove_const"), 1, ID2SYM(id)); }
VALUE rb_path2class(const char *path) {
  VALUE r = kn_capi_path2class(path);
  KN_CHECK(r);
  return r;
}
VALUE rb_path_to_class(VALUE path) { return rb_path2class(StringValueCStr(path)); }
VALUE rb_define_class_id_under(VALUE outer, ID name, VALUE super) { return rb_define_class_under(outer, rb_id2name(name), super); }
VALUE rb_define_module_id_under(VALUE outer, ID name) { return rb_define_module_under(outer, rb_id2name(name)); }
void rb_prepend_module(VALUE klass, VALUE module) { rb_funcall(klass, rb_intern("prepend"), 1, module); }
VALUE rb_class_name(VALUE c) {
  VALUE n = rb_funcall(c, rb_intern("name"), 0);
  return NIL_P(n) ? rb_funcall(c, rb_intern("to_s"), 0) : n;
}
VALUE rb_class_path(VALUE c) { return rb_class_name(c); }
VALUE rb_mod_name(VALUE c) { return rb_funcall(c, rb_intern("name"), 0); }
VALUE rb_class_superclass(VALUE c) { return rb_funcall(c, rb_intern("superclass"), 0); }
VALUE rb_class_get_superclass(VALUE c) { return rb_class_superclass(c); }
VALUE rb_class_inherited_p(VALUE mod, VALUE arg) { return rb_funcall(mod, rb_intern("<="), 1, arg); }
VALUE rb_mod_ancestors(VALUE mod) { return rb_funcall(mod, rb_intern("ancestors"), 0); }
VALUE rb_mod_include_p(VALUE mod, VALUE mod2) { return rb_funcall(mod, rb_intern("include?"), 1, mod2); }
VALUE rb_obj_dup(VALUE v) { return rb_funcall(v, rb_intern("dup"), 0); }
VALUE rb_obj_clone(VALUE v) { return rb_funcall(v, rb_intern("clone"), 0); }
VALUE rb_obj_id(VALUE v) { return rb_funcall(v, rb_intern("object_id"), 0); }
VALUE rb_obj_hide(VALUE v) { return v; }
VALUE rb_obj_reveal(VALUE v, VALUE klass) { (void)klass; return v; }
VALUE rb_obj_equal(VALUE a, VALUE b) { return a == b ? Qtrue : Qfalse; }
int rb_eql(VALUE a, VALUE b) { return a == b || RTEST(rb_funcall(a, rb_intern("eql?"), 1, b)); }
VALUE rb_obj_instance_variables(VALUE obj) { return rb_funcall(obj, rb_intern("instance_variables"), 0); }
VALUE rb_ivar_defined(VALUE obj, ID id) { return rb_funcall(obj, rb_intern("instance_variable_defined?"), 1, ID2SYM(id)); }
VALUE rb_cvar_get(VALUE klass, ID id) { return rb_funcall(klass, rb_intern("class_variable_get"), 1, ID2SYM(id)); }
void rb_cvar_set(VALUE klass, ID id, VALUE v) { rb_funcall(klass, rb_intern("class_variable_set"), 2, ID2SYM(id), v); }
VALUE rb_cvar_defined(VALUE klass, ID id) { return rb_funcall(klass, rb_intern("class_variable_defined?"), 1, ID2SYM(id)); }
VALUE rb_cv_get(VALUE klass, const char *name) { return rb_cvar_get(klass, rb_intern(name)); }
void rb_cv_set(VALUE klass, const char *name, VALUE v) { rb_cvar_set(klass, rb_intern(name), v); }
void rb_define_class_variable(VALUE klass, const char *name, VALUE v) { rb_cv_set(klass, name, v); }
void rb_ext_ractor_safe(bool flag) { (void)flag; }

/* ---------- data objects ---------- */

void *rb_check_typeddata(VALUE obj, const rb_data_type_t *type) {
  int ok;
  void *p = kn_capi_check_typeddata(obj, type, &ok);
  if (!ok) kn_unwind();
  return p;
}
int rb_typeddata_inherited_p(const rb_data_type_t *child, const rb_data_type_t *parent) {
  for (; child; child = child->parent)
    if (child == parent) return 1;
  return 0;
}
int rb_typeddata_is_kind_of(VALUE obj, const rb_data_type_t *type) {
  const rb_data_type_t *t = rb_kn_typeddata_type(obj);
  return t && rb_typeddata_inherited_p(t, type);
}
VALUE rb_data_object_zalloc(VALUE klass, size_t size, RUBY_DATA_FUNC mark, RUBY_DATA_FUNC free) {
  return rb_data_object_wrap(klass, ruby_xcalloc(1, size), mark, free);
}

/* ---------- memory ---------- */

void *ruby_xmalloc(size_t n) {
  void *p = malloc(n ? n : 1);
  if (!p) rb_memerror();
  return p;
}
void *ruby_xmalloc2(size_t n, size_t size) {
  if (size && n > SIZE_MAX / size) rb_raise(rb_eArgError, "malloc: possible integer overflow");
  return ruby_xmalloc(n * size);
}
void *ruby_xcalloc(size_t n, size_t size) {
  void *p = calloc(n ? n : 1, size ? size : 1);
  if (!p) rb_memerror();
  return p;
}
void *ruby_xrealloc(void *p, size_t n) {
  void *q = realloc(p, n ? n : 1);
  if (!q) rb_memerror();
  return q;
}
void *ruby_xrealloc2(void *p, size_t n, size_t size) {
  if (size && n > SIZE_MAX / size) rb_raise(rb_eArgError, "realloc: possible integer overflow");
  return ruby_xrealloc(p, n * size);
}
void ruby_xfree(void *p) { free(p); }
char *ruby_strdup(const char *s) {
  size_t n = strlen(s) + 1;
  char *p = (char *)ruby_xmalloc(n);
  memcpy(p, s, n);
  return p;
}

/* ---------- GC ---------- */

void rb_gc_mark_locations(const VALUE *start, const VALUE *end) {
  for (const VALUE *p = start; p < end; p++) rb_gc_mark_maybe(*p);
}
void rb_gc_mark_movable(VALUE v) { rb_gc_mark(v); }
VALUE rb_gc_location(VALUE v) { return v; }
void rb_gc_start(void) { rb_gc(); }
VALUE rb_gc_enable(void) { return rb_funcall(rb_mGC, rb_intern("enable"), 0); }
VALUE rb_gc_disable(void) { return rb_funcall(rb_mGC, rb_intern("disable"), 0); }
void rb_gc_adjust_memory_usage(ssize_t diff) { (void)diff; }
void rb_gc_writebarrier(VALUE a, VALUE b) { (void)a; (void)b; }
void rb_gc_writebarrier_unprotect(VALUE obj) { (void)obj; }
size_t rb_gc_count(void) { return NUM2SIZET(rb_funcall(rb_mGC, rb_intern("count"), 0)); }
int rb_during_gc(void) { return 0; }

/* ---------- threads, mutexes, time ---------- */

void *rb_thread_call_without_gvl(void *(*func)(void *), void *data1, void (*ubf)(void *), void *data2) {
  (void)ubf; (void)data2;
  return func(data1);
}
void *rb_thread_call_without_gvl2(void *(*func)(void *), void *data1, void (*ubf)(void *), void *data2) {
  return rb_thread_call_without_gvl(func, data1, ubf, data2);
}
void *rb_thread_call_with_gvl(void *(*func)(void *), void *data1) { return func(data1); }
void *rb_nogvl(void *(*func)(void *), void *data1, void (*ubf)(void *), void *data2, int flags) {
  (void)flags;
  return rb_thread_call_without_gvl(func, data1, ubf, data2);
}
VALUE rb_thread_current(void) { return rb_funcall(rb_cThread, rb_intern("current"), 0); }
VALUE rb_thread_main(void) { return rb_funcall(rb_cThread, rb_intern("main"), 0); }
void rb_thread_schedule(void) { rb_funcall(rb_cThread, rb_intern("pass"), 0); }
void rb_thread_check_ints(void) {}
int rb_thread_interrupted(VALUE th) { (void)th; return 0; }
VALUE rb_thread_local_aref(VALUE th, ID id) { return rb_funcall(th, rb_intern("[]"), 1, ID2SYM(id)); }
VALUE rb_thread_local_aset(VALUE th, ID id, VALUE v) { return rb_funcall(th, rb_intern("[]="), 2, ID2SYM(id), v); }
VALUE rb_mutex_new(void) { return rb_class_new_instance(0, NULL, rb_path2class("Thread::Mutex")); }
VALUE rb_mutex_lock(VALUE m) { return rb_funcall(m, rb_intern("lock"), 0); }
VALUE rb_mutex_unlock(VALUE m) { return rb_funcall(m, rb_intern("unlock"), 0); }
VALUE rb_mutex_trylock(VALUE m) { return rb_funcall(m, rb_intern("try_lock"), 0); }
VALUE rb_mutex_locked_p(VALUE m) { return rb_funcall(m, rb_intern("locked?"), 0); }
VALUE rb_mutex_synchronize(VALUE m, VALUE (*func)(VALUE), VALUE arg) {
  rb_mutex_lock(m);
  return rb_ensure(func, arg, rb_mutex_unlock, m);
}
VALUE rb_time_new(time_t sec, long usec) { return rb_funcall(rb_cTime, rb_intern("at"), 2, LL2NUM((long long)sec), LONG2NUM(usec)); }
VALUE rb_time_nano_new(time_t sec, long nsec) {
  return rb_funcall(rb_cTime, rb_intern("at"), 3, LL2NUM((long long)sec), LONG2NUM(nsec), ID2SYM(rb_intern("nsec")));
}
struct timespec rb_time_timespec(VALUE time) {
  struct timespec t;
  if (RTEST(rb_obj_is_kind_of(time, rb_cTime))) {
    t.tv_sec = (time_t)NUM2LL(rb_funcall(time, rb_intern("tv_sec"), 0));
    t.tv_nsec = NUM2LONG(rb_funcall(time, rb_intern("tv_nsec"), 0));
  } else {
    double d = NUM2DBL(time);
    t.tv_sec = (time_t)d;
    t.tv_nsec = (long)((d - (double)t.tv_sec) * 1e9);
  }
  return t;
}
struct timeval rb_time_timeval(VALUE time) {
  struct timespec ts = rb_time_timespec(time);
  struct timeval tv;
  tv.tv_sec = ts.tv_sec;
  tv.tv_usec = (int)(ts.tv_nsec / 1000);
  return tv;
}

/* ---------- argument parsing ---------- */

/* rb_scan_args: "<req><opt>*<post>:&" (a trailing Hash is the keywords only when it was
 * passed as keywords, as in Ruby 3) */
static int kn_scan_args(int kw_splat, int argc, const VALUE *argv, const char *fmt, va_list ap) {
  int req = 0, opt = 0, rest = 0, post = 0, kw = 0, blk = 0;
  const char *p = fmt;
  if (*p >= '0' && *p <= '9') req = *p++ - '0';
  if (*p >= '0' && *p <= '9') opt = *p++ - '0';
  if (*p == '*') { rest = 1; p++; }
  if (*p >= '0' && *p <= '9') post = *p++ - '0';
  if (*p == ':') { kw = 1; p++; }
  if (*p == '&') { blk = 1; p++; }
  VALUE hash = Qnil;
  if (kw && argc > 0 && RB_TYPE_P(argv[argc - 1], T_HASH) && (kw_splat || kn_capi_is_kwhash(argv[argc - 1]))) hash = argv[--argc];
  if (argc < req + post || (!rest && argc > req + opt + post)) {
    if (rest) rb_error_arity(argc, req + post, UNLIMITED_ARGUMENTS);
    rb_error_arity(argc, req + post, req + opt + post);
  }
  int i = 0;
  for (int k = 0; k < req; k++) { VALUE *v = va_arg(ap, VALUE *); if (v) *v = argv[i]; i++; }
  for (int k = 0; k < opt; k++) {
    VALUE *v = va_arg(ap, VALUE *);
    if (i < argc - post) { if (v) *v = argv[i]; i++; } else if (v) *v = Qnil;
  }
  if (rest) {
    VALUE *v = va_arg(ap, VALUE *);
    int n = argc - post - i;
    if (n < 0) n = 0;
    if (v) *v = rb_ary_new_from_values(n, argv + i);
    i += n;
  }
  for (int k = 0; k < post; k++) { VALUE *v = va_arg(ap, VALUE *); if (v) *v = argv[i]; i++; }
  if (kw) { VALUE *v = va_arg(ap, VALUE *); if (v) *v = hash; }
  if (blk) { VALUE *v = va_arg(ap, VALUE *); if (v) *v = rb_block_given_p() ? rb_block_proc() : Qnil; }
  return argc;
}
int rb_scan_args(int argc, const VALUE *argv, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int r = kn_scan_args(0, argc, argv, fmt, ap);
  va_end(ap);
  return r;
}
int rb_scan_args_kw(int kw_splat, int argc, const VALUE *argv, const char *fmt, ...) {
  va_list ap;
  va_start(ap, fmt);
  int r = kn_scan_args(kw_splat == 1, argc, argv, fmt, ap);
  va_end(ap);
  return r;
}
/* rb_get_kwargs: take the keywords table[0, required + optional) out of `hash` (into values,
 * Qundef for an absent optional one); others are an error unless optional < 0 (-1 - n
 * optional keywords, the rest allowed). Returns how many were found. */
int rb_get_kwargs(VALUE hash, const ID *table, int required, int optional, VALUE *values) {
  int rest = optional < 0;
  if (rest) optional = -1 - optional;
  int found = 0;
  VALUE missing = Qnil;
  for (int i = 0; i < required + optional; i++) {
    VALUE key = ID2SYM(table[i]);
    int has = !NIL_P(hash) && RTEST(rb_funcall(hash, rb_intern("key?"), 1, key));
    if (has) {
      found++;
      if (values) values[i] = rb_hash_delete(hash, key);
    } else {
      if (values) values[i] = Qundef;
      if (i < required) {
        if (NIL_P(missing)) missing = rb_ary_new();
        rb_ary_push(missing, key);
      }
    }
  }
  if (!NIL_P(missing)) {
    VALUE list = rb_funcall(rb_funcall(missing, rb_intern("map"), 0), rb_intern("join"), 1, rb_utf8_str_new_cstr(", "));
    (void)list;
    long n = RARRAY_LEN(missing);
    VALUE names = rb_utf8_str_new("", 0);
    for (long i = 0; i < n; i++) {
      if (i) rb_str_cat_cstr(names, ", ");
      rb_str_append(names, rb_inspect(rb_ary_entry(missing, i)));
    }
    rb_raise(rb_eArgError, "missing keyword%s: %" PRIsVALUE, n > 1 ? "s" : "", names);
  }
  if (!rest && !NIL_P(hash) && RHASH_SIZE(hash) > 0) {
    VALUE keys = rb_hash_keys(hash);
    long n = RARRAY_LEN(keys);
    VALUE names = rb_utf8_str_new("", 0);
    for (long i = 0; i < n; i++) {
      if (i) rb_str_cat_cstr(names, ", ");
      rb_str_append(names, rb_inspect(rb_ary_entry(keys, i)));
    }
    rb_raise(rb_eArgError, "unknown keyword%s: %" PRIsVALUE, n > 1 ? "s" : "", names);
  }
  return found;
}

/* (the global takes the variable's value when it is defined; later C assignments to the
 * variable are not seen by Ruby) */
void rb_define_variable(const char *name, VALUE *var) {
  rb_gc_register_address(var);
  rb_gv_set(name, *var);
}
void rb_define_readonly_variable(const char *name, const VALUE *var) { rb_define_variable(name, (VALUE *)var); }
ID rb_intern3(const char *name, long len, rb_encoding *enc) { (void)enc; return rb_intern2(name, len); }
