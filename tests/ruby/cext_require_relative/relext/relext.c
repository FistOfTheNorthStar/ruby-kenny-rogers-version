#include <ruby.h>
static VALUE hi(VALUE self) { return rb_str_new_cstr("hi from relext"); }
void Init_relext(void) { rb_define_global_function("relext_hi", hi, 0); }
