/* The C API's st_table and rb_fdset_t, driven from tests/ruby/capi_st.rb. */
#include <ruby.h>
#include <ruby/st.h>
#include <ruby/thread.h>
#include <unistd.h>

static int sum_i(st_data_t k, st_data_t v, st_data_t arg) {
  *(long *)arg += (long)k * (long)v;
  return ST_CONTINUE;
}
static int drop_odd_i(st_data_t k, st_data_t v, st_data_t arg) {
  (void)v; (void)arg;
  return (k & 1) ? ST_DELETE : ST_CONTINUE;
}
static int grow_i(st_data_t k, st_data_t v, st_data_t arg) {
  /* adding during the walk: the walk goes on over them */
  st_table *t = (st_table *)arg;
  (void)v;
  if (k < 50) st_insert(t, k + 100, 1);
  return ST_CONTINUE;
}
static int stop_i(st_data_t k, st_data_t v, st_data_t arg) {
  (void)v;
  *(long *)arg = (long)k;
  return k == 6 ? ST_STOP : ST_CONTINUE;
}
static int bump_i(st_data_t *k, st_data_t *v, st_data_t arg, int existing) {
  (void)k;
  *v = existing ? *v + arg : arg;
  return ST_CONTINUE;
}
static int gone_i(st_data_t *k, st_data_t *v, st_data_t arg, int existing) {
  (void)k; (void)v; (void)arg; (void)existing;
  return ST_DELETE;
}
static VALUE num_table(VALUE self) {
  st_table *t = st_init_numtable();
  VALUE out = rb_ary_new();
  st_data_t v = 0, k;
  long sum = 0, last = 0;
  for (long i = 1; i <= 1000; i++) st_insert(t, (st_data_t)i, (st_data_t)(i * 2));
  rb_ary_push(out, LONG2NUM((long)t->num_entries));
  rb_ary_push(out, st_insert(t, 5, 11) ? Qtrue : Qfalse);
  rb_ary_push(out, st_lookup(t, 5, &v) ? LONG2NUM((long)v) : Qnil);
  rb_ary_push(out, st_lookup(t, 5000, &v) ? Qtrue : Qfalse);
  k = 5;
  rb_ary_push(out, st_delete(t, &k, &v) ? LONG2NUM((long)v) : Qnil);
  rb_ary_push(out, st_lookup(t, 5, &v) ? Qtrue : Qfalse);
  st_foreach(t, sum_i, (st_data_t)&sum);
  rb_ary_push(out, LONG2NUM(sum));
  st_foreach(t, drop_odd_i, 0);
  rb_ary_push(out, LONG2NUM((long)t->num_entries));
  st_foreach(t, stop_i, (st_data_t)&last);
  rb_ary_push(out, LONG2NUM(last));
  st_update(t, 2, bump_i, 10);
  st_update(t, 7777, bump_i, 3);
  st_update(t, 4, gone_i, 0);
  st_lookup(t, 2, &v);
  rb_ary_push(out, LONG2NUM((long)v));
  st_lookup(t, 7777, &v);
  rb_ary_push(out, LONG2NUM((long)v));
  rb_ary_push(out, st_lookup(t, 4, &v) ? Qtrue : Qfalse);
  st_table *c = st_copy(t);
  st_clear(t);
  rb_ary_push(out, LONG2NUM((long)t->num_entries));
  rb_ary_push(out, LONG2NUM((long)c->num_entries));
  st_data_t first[3];
  rb_ary_push(out, LONG2NUM((long)st_keys(c, first, 3)));
  rb_ary_push(out, LONG2NUM((long)first[2]));
  st_free_table(c);
  st_free_table(t);
  /* churn: tombstones must not fill the table */
  t = st_init_numtable();
  for (long i = 0; i < 200000; i++) {
    st_insert(t, (st_data_t)i, 1);
    k = (st_data_t)i;
    st_delete(t, &k, NULL);
  }
  rb_ary_push(out, LONG2NUM((long)t->num_entries));
  /* adding while walking */
  for (long i = 0; i < 50; i++) st_insert(t, (st_data_t)i, 1);
  k = 3;
  st_delete(t, &k, NULL);
  st_foreach(t, grow_i, (st_data_t)t);
  rb_ary_push(out, LONG2NUM((long)t->num_entries));
  st_free_table(t);
  return out;
}
static VALUE str_table(VALUE self) {
  st_table *t = st_init_strtable();
  st_table *ci = st_init_strcasetable();
  st_data_t v = 0;
  VALUE out = rb_ary_new();
  char a[8] = "alpha", b[8] = "alpha";
  st_insert(t, (st_data_t)a, 1);
  rb_ary_push(out, st_lookup(t, (st_data_t)b, &v) ? LONG2NUM((long)v) : Qnil);
  rb_ary_push(out, st_lookup(t, (st_data_t)"ALPHA", &v) ? Qtrue : Qfalse);
  st_insert(ci, (st_data_t)"Header", 7);
  rb_ary_push(out, st_lookup(ci, (st_data_t)"hEADER", &v) ? LONG2NUM((long)v) : Qnil);
  st_free_table(t);
  st_free_table(ci);
  return out;
}
static VALUE fd_select(VALUE self) {
  int p[2];
  VALUE out = rb_ary_new();
  if (pipe(p) != 0) rb_sys_fail("pipe");
  rb_fdset_t r;
  rb_fd_init(&r);
  rb_fd_set(p[0], &r);
  rb_ary_push(out, rb_fd_isset(p[0], &r) ? Qtrue : Qfalse);
  rb_ary_push(out, rb_fd_isset(p[1], &r) ? Qtrue : Qfalse);
  struct timeval tv = {0, 10000};
  rb_ary_push(out, INT2NUM(rb_thread_fd_select(rb_fd_max(&r), &r, NULL, NULL, &tv)));
  rb_ary_push(out, rb_fd_isset(p[0], &r) ? Qtrue : Qfalse);
  if (write(p[1], "x", 1) != 1) rb_sys_fail("write");
  rb_fd_set(p[0], &r);
  struct timeval tv2 = {5, 0};
  rb_ary_push(out, INT2NUM(rb_thread_fd_select(rb_fd_max(&r), &r, NULL, NULL, &tv2)));
  rb_ary_push(out, rb_fd_isset(p[0], &r) ? Qtrue : Qfalse);
  rb_fd_clr(p[0], &r);
  rb_ary_push(out, rb_fd_isset(p[0], &r) ? Qtrue : Qfalse);
  rb_fd_term(&r);
  close(p[0]);
  close(p[1]);
  return out;
}
void Init_capi_st(void) {
  VALUE m = rb_define_module("CapiSt");
  rb_define_module_function(m, "num_table", num_table, 0);
  rb_define_module_function(m, "str_table", str_table, 0);
  rb_define_module_function(m, "fd_select", fd_select, 0);
}
