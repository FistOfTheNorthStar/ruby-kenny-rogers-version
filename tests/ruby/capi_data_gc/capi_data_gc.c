/* Short-lived data objects with a mark function: the kind a minor collection frees. Counts
 * how often the collector marks one it has already freed (the structs are never returned to
 * malloc, so counting is safe). */
#include <ruby.h>

typedef struct { VALUE held; int freed; } box_t;

static long marked_after_free, freed_count;

static void box_mark(void *p) {
    box_t *b = p;
    if (b->freed) marked_after_free++;
    else rb_gc_mark(b->held);
}
static void box_free(void *p) { ((box_t *)p)->freed = 1; freed_count++; }
static const rb_data_type_t box_type = { "Box", { box_mark, box_free, 0 }, 0, 0, RUBY_TYPED_FREE_IMMEDIATELY };

static VALUE box_alloc(VALUE klass) {
    box_t *b;
    VALUE v = TypedData_Make_Struct(klass, box_t, &box_type, b);
    b->held = Qnil;
    return v;
}
static VALUE box_set(VALUE self, VALUE x) {
    box_t *b;
    TypedData_Get_Struct(self, box_t, &box_type, b);
    b->held = x;
    return self;
}
static VALUE box_get(VALUE self) {
    box_t *b;
    TypedData_Get_Struct(self, box_t, &box_type, b);
    return b->held;
}
static VALUE box_marked_after_free(VALUE self) { (void)self; return LONG2NUM(marked_after_free); }
static VALUE box_freed_count(VALUE self) { (void)self; return LONG2NUM(freed_count); }

void Init_capi_data_gc(void) {
    VALUE c = rb_define_class("Box", rb_cObject);
    rb_define_alloc_func(c, box_alloc);
    rb_define_method(c, "set", box_set, 1);
    rb_define_method(c, "get", box_get, 0);
    rb_define_singleton_method(c, "marked_after_free", box_marked_after_free, 0);
    rb_define_singleton_method(c, "freed_count", box_freed_count, 0);
}
