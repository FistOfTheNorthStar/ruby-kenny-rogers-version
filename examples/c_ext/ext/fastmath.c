/* A tiny C extension written against the standard Ruby C API. */
#include <ruby.h>

typedef struct { double sum; long count; } Stats;

static void stats_free(void *p) { free(p); }
static const rb_data_type_t stats_type = {"Stats", {NULL, stats_free, NULL}, NULL, NULL, RUBY_TYPED_FREE_IMMEDIATELY};

static VALUE stats_alloc(VALUE klass) {
  Stats *s;
  return TypedData_Make_Struct(klass, Stats, &stats_type, s);
}
static VALUE stats_add(VALUE self, VALUE x) {
  Stats *s;
  TypedData_Get_Struct(self, Stats, &stats_type, s);
  s->sum += NUM2DBL(x);
  s->count++;
  return self;
}
static VALUE stats_mean(VALUE self) {
  Stats *s;
  TypedData_Get_Struct(self, Stats, &stats_type, s);
  return s->count ? DBL2NUM(s->sum / s->count) : Qnil;
}

static VALUE fm_fib(VALUE self, VALUE n) {
  long k = NUM2LONG(n), a = 0, b = 1;
  if (k < 0) rb_raise(rb_eArgError, "negative index %ld", k);
  for (long i = 0; i < k; i++) { long t = a + b; a = b; b = t; }
  return LONG2NUM(a);
}

static VALUE fm_sum(int argc, VALUE *argv, VALUE self) {
  VALUE rest, opts;
  rb_scan_args(argc, argv, "*:", &rest, &opts);
  double total = 0;
  for (long i = 0; i < RARRAY_LEN(rest); i++) total += NUM2DBL(rb_ary_entry(rest, i));
  if (!NIL_P(opts)) {
    VALUE scale = rb_hash_aref(opts, ID2SYM(rb_intern("scale")));
    if (!NIL_P(scale)) total *= NUM2DBL(scale);
  }
  return DBL2NUM(total);
}

static VALUE fm_shout(VALUE self, VALUE str) {
  StringValue(str);
  VALUE out = rb_str_dup(str);
  char *p = RSTRING_PTR(out);
  for (long i = 0; i < RSTRING_LEN(out); i++) if (p[i] >= 'a' && p[i] <= 'z') p[i] -= 32;
  return rb_str_cat_cstr(out, "!");
}

static VALUE fm_each_square(VALUE self, VALUE n) {
  VALUE out = rb_ary_new();
  for (long i = 1; i <= NUM2LONG(n); i++) rb_ary_push(out, rb_yield(LONG2NUM(i * i)));
  return out;
}

static VALUE fm_call_back(VALUE self, VALUE obj) {
  return rb_funcall(obj, rb_intern("upcase"), 0);
}

void Init_fastmath(void) {
  VALUE mod = rb_define_module("FastMath");
  rb_define_module_function(mod, "fib", RUBY_METHOD_FUNC(fm_fib), 1);
  rb_define_module_function(mod, "sum", RUBY_METHOD_FUNC(fm_sum), -1);
  rb_define_module_function(mod, "shout", RUBY_METHOD_FUNC(fm_shout), 1);
  rb_define_module_function(mod, "each_square", RUBY_METHOD_FUNC(fm_each_square), 1);
  rb_define_module_function(mod, "call_back", RUBY_METHOD_FUNC(fm_call_back), 1);
  rb_define_const(mod, "VERSION", rb_str_new_cstr("1.0"));
  VALUE stats = rb_define_class_under(mod, "Stats", rb_cObject);
  rb_define_alloc_func(stats, stats_alloc);
  rb_define_method(stats, "add", RUBY_METHOD_FUNC(stats_add), 1);
  rb_define_method(stats, "mean", RUBY_METHOD_FUNC(stats_mean), 0);
}
