/* C methods that run Ruby in the middle: other fibers and threads run C methods meanwhile. */
#include <ruby.h>

static VALUE yield_then_raise(VALUE self, VALUE tag) {
  rb_yield(Qnil);
  rb_raise(rb_eArgError, "raised for %ld", NUM2LONG(tag));
  return Qnil;
}
static VALUE yield_then_name(VALUE self) {
  rb_yield(Qnil);
  return ID2SYM(rb_frame_this_func());
}
static VALUE plain(VALUE self) { return self; }
static VALUE yield_arg(VALUE arg) { return rb_yield(arg); }
static VALUE protected_yield(VALUE self) {
  int state = 0;
  rb_protect(yield_arg, Qnil, &state);
  rb_set_errinfo(Qnil);
  return INT2NUM(state != 0);
}
void Init_capi_context(void) {
  VALUE m = rb_define_module("CapiContext");
  rb_define_singleton_method(m, "yield_then_raise", yield_then_raise, 1);
  rb_define_singleton_method(m, "yield_then_name", yield_then_name, 0);
  rb_define_singleton_method(m, "plain", plain, 0);
  rb_define_singleton_method(m, "protected_yield", protected_yield, 0);
}
