/* A C extension's data objects with mark and free functions (see gc_c_data.rb). The free
 * function poisons the struct and keeps it (so a later call of the mark function on it is
 * detected instead of being a read of freed memory). */
#include "ruby.h"
#include <stdio.h>
#include <stdlib.h>

#define LIVE 0x11a1u
#define DEAD 0xdeadu

struct box {
  unsigned magic;
  VALUE ref;
};

static long freed_count;

static void box_mark(void *p) {
  struct box *b = p;
  if (b->magic != LIVE) {
    fprintf(stderr, "mark function called on freed data (magic %#x)\n", b->magic);
    fflush(stderr);
    abort();
  }
  rb_gc_mark(b->ref);
}

static void box_free(void *p) {
  struct box *b = p;
  b->magic = DEAD;
  freed_count++;
}

static const rb_data_type_t box_type = {
  "GcCData::Box",
  { box_mark, box_free, NULL, },
  NULL, NULL, 0,
};

static VALUE box_alloc(VALUE klass) {
  struct box *b = malloc(sizeof(struct box));
  b->magic = LIVE;
  b->ref = Qnil;
  return TypedData_Wrap_Struct(klass, &box_type, b);
}

static VALUE box_init(VALUE self, VALUE ref) {
  struct box *b;
  TypedData_Get_Struct(self, struct box, &box_type, b);
  b->ref = ref;
  return self;
}

static VALUE box_ref(VALUE self) {
  struct box *b;
  TypedData_Get_Struct(self, struct box, &box_type, b);
  return b->ref;
}

static VALUE freed(VALUE self) {
  return freed_count > 0 ? Qtrue : Qfalse;
}

void Init_gc_c_data(void) {
  VALUE m = rb_define_module("GcCData");
  VALUE c = rb_define_class_under(m, "Box", rb_cObject);
  rb_define_alloc_func(c, box_alloc);
  rb_define_method(c, "initialize", box_init, 1);
  rb_define_method(c, "ref", box_ref, 0);
  rb_define_singleton_method(m, "freed?", freed, 0);
}
