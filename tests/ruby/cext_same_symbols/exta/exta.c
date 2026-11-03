#include <ruby.h>
int bufgrow(int n);
extern const char *buf_owner;
static VALUE m(VALUE self, VALUE n) { return rb_ary_new_from_args(2, rb_str_new_cstr(buf_owner), INT2FIX(bufgrow(FIX2INT(n)))); }
void Init_exta(void) { rb_define_global_function("ext_a", m, 1); }
