/* Kenny C-API shim: the parts of the CRuby API that need setjmp/longjmp or varargs.
 * Exceptions raised while a C method runs longjmp back to that method's call boundary
 * (kn_call), exactly like CRuby. The longjmp only ever crosses C frames. */
#include "ruby.h"
#include <setjmp.h>
#include <stdarg.h>
#include <stdio.h>

/* implemented in the Rust runtime */
typedef VALUE (*kn_cfunc_caller)(const void *, int, VALUE, int, const VALUE *);
void kn_capi_register_caller(kn_cfunc_caller f);
VALUE kn_capi_funcallv(VALUE recv, ID mid, int argc, const VALUE *argv);
VALUE kn_capi_funcall_with_block(VALUE recv, ID mid, int argc, const VALUE *argv, VALUE blk);
VALUE kn_capi_yield_values(int argc, const VALUE *argv);
VALUE kn_capi_raise(VALUE klass, const char *msg);
VALUE kn_capi_exc_raise(VALUE e);
VALUE kn_capi_take_pending(void);
VALUE kn_capi_check_type(VALUE v, int t);
VALUE kn_capi_check_frozen(VALUE v);
long kn_capi_num2long(VALUE v, int *ok);
double kn_capi_num2dbl(VALUE v, int *ok);
VALUE kn_capi_string_value(VALUE v);
VALUE kn_capi_inspect(VALUE v);
VALUE kn_capi_hash_aref(VALUE h, VALUE k);
VALUE kn_capi_class_new_instance(int argc, const VALUE *argv, VALUE klass);
VALUE kn_capi_const_get(VALUE klass, ID id);
VALUE kn_capi_path2class(const char *path);
VALUE kn_capi_obj_alloc(VALUE klass);
void *kn_capi_check_typeddata(VALUE obj, const rb_data_type_t *type);
void kn_capi_warn(const char *msg);

#define KN_MAX_DEPTH 4096
static jmp_buf *kn_jmp_stack[KN_MAX_DEPTH];
static int kn_jmp_depth = 0;

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
typedef VALUE (*fv)(int, const VALUE *, VALUE);
typedef VALUE (*fa)(VALUE, VALUE);

static VALUE kn_call(const void *f, int arity, VALUE self, int argc, const VALUE *a) {
  jmp_buf jb;
  volatile VALUE r = Qundef;
  if (kn_jmp_depth >= KN_MAX_DEPTH) return kn_capi_raise(rb_eRuntimeError, "C call stack too deep");
  kn_jmp_stack[kn_jmp_depth++] = &jb;
  if (setjmp(jb) == 0) {
    switch (arity) {
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
      default: r = kn_capi_raise(rb_eNotImpError, "C methods with more than 8 fixed arguments"); break;
    }
  } else {
    r = Qundef; /* an exception is pending in the runtime */
  }
  kn_jmp_depth--;
  return r;
}

__attribute__((constructor)) static void kn_capi_init(void) { kn_capi_register_caller(kn_call); }

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
VALUE rb_funcall_with_block(VALUE recv, ID mid, int argc, const VALUE *argv, VALUE blk) {
  VALUE r = kn_capi_funcall_with_block(recv, mid, argc, argv, blk);
  KN_CHECK(r);
  return r;
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
VALUE rb_ary_new_from_args(long n, ...) {
  VALUE a = rb_ary_new_capa(n);
  va_list ap;
  va_start(ap, n);
  for (long i = 0; i < n; i++) rb_ary_push(a, va_arg(ap, VALUE));
  va_end(ap);
  return a;
}

void rb_raise(VALUE klass, const char *fmt, ...) {
  char buf[2048];
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(buf, sizeof buf, fmt, ap);
  va_end(ap);
  kn_capi_raise(klass, buf);
  kn_unwind();
}
void rb_exc_raise(VALUE e) {
  kn_capi_exc_raise(e);
  kn_unwind();
}
void rb_notimplement(void) { rb_raise(rb_eNotImpError, "function is unimplemented on this machine"); }
void rb_jump_tag(int state) {
  if (state) rb_exc_raise(rb_errinfo());
  kn_unwind();
}
void rb_warn(const char *fmt, ...) {
  char buf[2048];
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(buf, sizeof buf, fmt, ap);
  va_end(ap);
  kn_capi_warn(buf);
}
void rb_warning(const char *fmt, ...) { (void)fmt; }

void rb_check_type(VALUE v, int t) { KN_CHECK(kn_capi_check_type(v, t)); }
void rb_check_frozen(VALUE v) { KN_CHECK(kn_capi_check_frozen(v)); }
long rb_num2long(VALUE v) {
  if (FIXNUM_P(v)) return FIX2LONG(v);
  int ok;
  long r = kn_capi_num2long(v, &ok);
  if (!ok) kn_unwind();
  return r;
}
unsigned long rb_num2ulong(VALUE v) { return (unsigned long)rb_num2long(v); }
int rb_num2int(VALUE v) { return (int)rb_num2long(v); }
double rb_num2dbl(VALUE v) {
  int ok;
  double r = kn_capi_num2dbl(v, &ok);
  if (!ok) kn_unwind();
  return r;
}
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
VALUE rb_inspect(VALUE v) {
  VALUE r = kn_capi_inspect(v);
  KN_CHECK(r);
  return r;
}
VALUE rb_hash_aref(VALUE h, VALUE k) {
  VALUE r = kn_capi_hash_aref(h, k);
  KN_CHECK(r);
  return r;
}
VALUE rb_class_new_instance(int argc, const VALUE *argv, VALUE klass) {
  VALUE r = kn_capi_class_new_instance(argc, argv, klass);
  KN_CHECK(r);
  return r;
}
VALUE rb_obj_alloc(VALUE klass) {
  VALUE r = kn_capi_obj_alloc(klass);
  KN_CHECK(r);
  return r;
}
VALUE rb_const_get(VALUE klass, ID id) {
  VALUE r = kn_capi_const_get(klass, id);
  KN_CHECK(r);
  return r;
}
VALUE rb_path2class(const char *path) {
  VALUE r = kn_capi_path2class(path);
  KN_CHECK(r);
  return r;
}
void *rb_check_typeddata(VALUE obj, const rb_data_type_t *type) {
  void *p = kn_capi_check_typeddata(obj, type);
  if (!p && RB_TYPE_P(obj, T_DATA) == 0) kn_unwind();
  return p;
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
  kn_capi_take_pending();
  if (state) *state = 6; /* TAG_RAISE */
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
  VALUE e = kn_capi_take_pending();
  e_proc(data2);
  rb_exc_raise(e);
}
static VALUE kn_rescue(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2, VALUE klass) {
  jmp_buf jb;
  volatile VALUE r = Qnil;
  kn_jmp_stack[kn_jmp_depth++] = &jb;
  if (setjmp(jb) == 0) {
    r = b_proc(data1);
    kn_jmp_depth--;
    return r;
  }
  kn_jmp_depth--;
  VALUE e = kn_capi_take_pending();
  if (RTEST(rb_obj_is_kind_of(e, klass))) return r_proc ? r_proc(data2, e) : Qnil;
  rb_exc_raise(e);
}
VALUE rb_rescue(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2) {
  return kn_rescue(b_proc, data1, r_proc, data2, rb_eStandardError);
}
VALUE rb_rescue2(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2, ...) {
  va_list ap;
  va_start(ap, data2);
  VALUE klass = va_arg(ap, VALUE);
  va_end(ap);
  return kn_rescue(b_proc, data1, r_proc, data2, klass ? klass : rb_eStandardError);
}

/* rb_scan_args: "12", "*", ":", "&" (the common subset) */
int rb_scan_args(int argc, const VALUE *argv, const char *fmt, ...) {
  int req = 0, opt = 0, rest = 0, post = 0, kw = 0, blk = 0;
  const char *p = fmt;
  if (*p >= '0' && *p <= '9') req = *p++ - '0';
  if (*p >= '0' && *p <= '9') opt = *p++ - '0';
  if (*p == '*') { rest = 1; p++; }
  if (*p >= '0' && *p <= '9') post = *p++ - '0';
  if (*p == ':') { kw = 1; p++; }
  if (*p == '&') { blk = 1; p++; }
  VALUE hash = Qnil;
  if (kw && argc > 0 && RB_TYPE_P(argv[argc - 1], T_HASH) && argc > req + post) hash = argv[--argc];
  if (argc < req + post || (!rest && argc > req + opt + post))
    rb_raise(rb_eArgError, "wrong number of arguments (given %d, expected %d%s)", argc, req + post, rest ? "+" : opt ? "+" : "");
  va_list ap;
  va_start(ap, fmt);
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
  va_end(ap);
  return argc;
}
