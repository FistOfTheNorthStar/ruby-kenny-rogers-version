/* Kenny's CRuby-compatible C API. Compile extensions with -std=gnu17. */
#ifndef KENNY_RUBY_H
#define KENNY_RUBY_H 1

#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef uintptr_t VALUE;
typedef uintptr_t ID;
typedef intptr_t SIGNED_VALUE;

#define RUBY_EXTERN extern
#define ANYARGS
#define RUBY_METHOD_FUNC(f) ((VALUE (*)())(f))
#define RUBY_FUNC_EXPORTED __attribute__((visibility("default")))
#define NORETURN(x) __attribute__((noreturn)) x
#define RUBY_API_VERSION_MAJOR 3
#define RUBY_API_VERSION_MINOR 4
#define HAVE_RB_DATA_TYPE_T_FUNCTION 1
#define HAVE_TYPE_RB_DATA_TYPE_T 1
#define HAVE_RB_ARY_NEW_CAPA 1

/* immediates: same tagging as CRuby */
#define Qfalse ((VALUE)0x00)
#define Qnil ((VALUE)0x08)
#define Qtrue ((VALUE)0x14)
#define Qundef ((VALUE)0x34)

#define RTEST(v) ((((VALUE)(v)) & ~Qnil) != 0)
#define NIL_P(v) ((VALUE)(v) == Qnil)
#define FIXNUM_P(v) ((((VALUE)(v)) & 1) == 1)
#define SYMBOL_P(v) ((((VALUE)(v)) & 0xff) == 0x0c)
/* flonums: Floats stored in the VALUE itself (low bits 0b10), as in 64-bit CRuby */
#define RUBY_FLONUM_MASK 0x03
#define RUBY_FLONUM_FLAG 0x02
#define FLONUM_P(v) ((((VALUE)(v)) & RUBY_FLONUM_MASK) == RUBY_FLONUM_FLAG)
#define USE_FLONUM 1
#define SPECIAL_CONST_P(v) (FIXNUM_P(v) || FLONUM_P(v) || SYMBOL_P(v) || (v) == Qnil || (v) == Qtrue || (v) == Qfalse)
#define IMMEDIATE_P(v) (FIXNUM_P(v) || FLONUM_P(v) || SYMBOL_P(v) || (v) == Qtrue)
#define INT2FIX(i) ((VALUE)((((SIGNED_VALUE)(i)) << 1) | 1))
#define LONG2FIX(i) INT2FIX(i)
#define FIX2LONG(v) ((long)(((SIGNED_VALUE)(v)) >> 1))
#define FIX2INT(v) ((int)FIX2LONG(v))
#define FIX2ULONG(v) ((unsigned long)FIX2LONG(v))
#define POSFIXABLE(n) ((n) < (1L << 62))
#define NEGFIXABLE(n) ((n) >= -(1L << 62))
#define FIXABLE(n) (POSFIXABLE(n) && NEGFIXABLE(n))

enum ruby_value_type {
  T_NONE = 0x00, T_OBJECT = 0x01, T_CLASS = 0x02, T_MODULE = 0x03, T_FLOAT = 0x04,
  T_STRING = 0x05, T_REGEXP = 0x06, T_ARRAY = 0x07, T_HASH = 0x08, T_STRUCT = 0x09,
  T_BIGNUM = 0x0a, T_FILE = 0x0b, T_DATA = 0x0c, T_MATCH = 0x0d, T_COMPLEX = 0x0e,
  T_RATIONAL = 0x0f, T_NIL = 0x11, T_TRUE = 0x12, T_FALSE = 0x13, T_SYMBOL = 0x14,
  T_FIXNUM = 0x15, T_UNDEF = 0x16
};

int rb_type(VALUE v);
#define TYPE(v) rb_type((VALUE)(v))
#define RB_TYPE_P(v, t) (rb_type((VALUE)(v)) == (t))
void rb_check_type(VALUE v, int t);
#define Check_Type(v, t) rb_check_type((VALUE)(v), (t))

/* class / exception globals */
RUBY_EXTERN VALUE rb_cObject, rb_cBasicObject, rb_cModule, rb_cClass, rb_mKernel, rb_mComparable,
    rb_mEnumerable, rb_cString, rb_cSymbol, rb_cArray, rb_cHash, rb_cInteger, rb_cFloat, rb_cNumeric,
    rb_cRange, rb_cProc, rb_cNilClass, rb_cTrueClass, rb_cFalseClass, rb_cStruct, rb_cTime, rb_cIO;
RUBY_EXTERN VALUE rb_eException, rb_eStandardError, rb_eRuntimeError, rb_eArgError, rb_eTypeError,
    rb_eNameError, rb_eNoMethodError, rb_eIndexError, rb_eKeyError, rb_eRangeError, rb_eZeroDivError,
    rb_eNotImpError, rb_eIOError, rb_eStopIteration, rb_eFrozenError, rb_eNoMemError, rb_eScriptError,
    rb_eFloatDomainError, rb_eEncodingError, rb_eLocalJumpError, rb_eSecurityError;
#define rb_cFixnum rb_cInteger
#define rb_cBignum rb_cInteger

/* definitions */
VALUE rb_define_module(const char *name);
VALUE rb_define_module_under(VALUE outer, const char *name);
VALUE rb_define_class(const char *name, VALUE super);
VALUE rb_define_class_under(VALUE outer, const char *name, VALUE super);
void rb_define_method(VALUE klass, const char *name, VALUE (*func)(), int argc);
void rb_define_private_method(VALUE klass, const char *name, VALUE (*func)(), int argc);
void rb_define_protected_method(VALUE klass, const char *name, VALUE (*func)(), int argc);
void rb_define_singleton_method(VALUE obj, const char *name, VALUE (*func)(), int argc);
void rb_define_module_function(VALUE mod, const char *name, VALUE (*func)(), int argc);
void rb_define_global_function(const char *name, VALUE (*func)(), int argc);
void rb_define_alias(VALUE klass, const char *new_name, const char *old_name);
void rb_define_attr(VALUE klass, const char *name, int read, int write);
void rb_define_const(VALUE klass, const char *name, VALUE val);
void rb_define_global_const(const char *name, VALUE val);
void rb_define_alloc_func(VALUE klass, VALUE (*func)(VALUE));
void rb_undef_alloc_func(VALUE klass);
void rb_include_module(VALUE klass, VALUE module);
void rb_extend_object(VALUE obj, VALUE module);

/* symbols */
ID rb_intern(const char *name);
ID rb_intern2(const char *name, long len);
#define rb_intern_const(s) rb_intern(s)
VALUE rb_id2sym(ID id);
ID rb_sym2id(VALUE sym);
const char *rb_id2name(ID id);
ID rb_to_id(VALUE v);
#define ID2SYM(id) rb_id2sym(id)
#define SYM2ID(s) rb_sym2id(s)

/* numbers */
VALUE rb_int2inum(intptr_t n);
VALUE rb_uint2inum(uintptr_t n);
VALUE rb_ll2inum(long long n);
VALUE rb_ull2inum(unsigned long long n);
VALUE rb_float_new(double f);
long rb_num2long(VALUE v);
unsigned long rb_num2ulong(VALUE v);
int rb_num2int(VALUE v);
double rb_num2dbl(VALUE v);
#define INT2NUM(i) rb_int2inum((intptr_t)(i))
#define LONG2NUM(i) rb_int2inum((intptr_t)(i))
#define UINT2NUM(i) rb_uint2inum((uintptr_t)(i))
#define ULONG2NUM(i) rb_uint2inum((uintptr_t)(i))
#define LL2NUM(i) rb_ll2inum(i)
#define ULL2NUM(i) rb_ull2inum(i)
#define SIZET2NUM(i) rb_uint2inum((uintptr_t)(i))
#define NUM2LONG(v) rb_num2long(v)
#define NUM2INT(v) rb_num2int(v)
#define NUM2UINT(v) ((unsigned int)rb_num2ulong(v))
#define NUM2ULONG(v) rb_num2ulong(v)
#define NUM2LL(v) ((long long)rb_num2long(v))
#define NUM2SIZET(v) ((size_t)rb_num2ulong(v))
#define NUM2DBL(v) rb_num2dbl(v)
#define DBL2NUM(f) rb_float_new(f)
#define RFLOAT_VALUE(v) rb_float_value(v)
#define RB_FLOAT_TYPE_P(v) (FLONUM_P(v) || RB_TYPE_P(v, T_FLOAT))
/* rb_float_new / rb_float_value: flonum (inline) or heap Float, like CRuby's */
static inline double rb_float_flonum_value(VALUE v) {
  if (v != (VALUE)0x8000000000000002) {
    VALUE b63 = v >> 63;
    union { double d; VALUE v; } t;
    VALUE x = (2 - b63) | (v & ~(VALUE)0x03);
    t.v = (x >> 3) | (x << 61);
    return t.d;
  }
  return 0.0;
}
static inline double rb_float_value(VALUE v) {
  return FLONUM_P(v) ? rb_float_flonum_value(v) : rb_num2dbl(v);
}

/* strings */
VALUE rb_str_new(const char *p, long len);
VALUE rb_str_new_cstr(const char *p);
VALUE rb_utf8_str_new(const char *p, long len);
VALUE rb_utf8_str_new_cstr(const char *p);
#define rb_str_new2 rb_str_new_cstr
#define rb_str_new_literal(s) rb_str_new((s), (long)(sizeof(s) - 1))
#define rb_usascii_str_new rb_str_new
#define rb_usascii_str_new_cstr rb_str_new_cstr
#define rb_external_str_new rb_str_new
#define rb_tainted_str_new rb_str_new
VALUE rb_str_cat(VALUE s, const char *p, long len);
VALUE rb_str_cat_cstr(VALUE s, const char *p);
#define rb_str_cat2 rb_str_cat_cstr
VALUE rb_str_append(VALUE s, VALUE o);
#define rb_str_concat rb_str_append
VALUE rb_str_dup(VALUE s);
VALUE rb_str_resize(VALUE s, long len);
void rb_str_set_len(VALUE s, long len);
VALUE rb_str_buf_new(long capa);
void rb_str_modify(VALUE s);
void rb_str_modify_expand(VALUE s, long extra);
VALUE rb_str_equal(VALUE a, VALUE b);
VALUE rb_str_intern(VALUE s);
VALUE rb_obj_as_string(VALUE v);
#define rb_String rb_obj_as_string
VALUE rb_inspect(VALUE v);
char *rb_kn_rstring_ptr(VALUE s);
long rb_kn_rstring_len(VALUE s);
#define RSTRING_PTR(s) rb_kn_rstring_ptr((VALUE)(s))
#define RSTRING_LEN(s) rb_kn_rstring_len((VALUE)(s))
#define RSTRING_END(s) (RSTRING_PTR(s) + RSTRING_LEN(s))
#define RSTRING_LENINT(s) ((int)RSTRING_LEN(s))
VALUE rb_string_value(volatile VALUE *ptr);
char *rb_string_value_ptr(volatile VALUE *ptr);
char *rb_string_value_cstr(volatile VALUE *ptr);
#define StringValue(v) rb_string_value(&(v))
#define StringValuePtr(v) rb_string_value_ptr(&(v))
#define StringValueCStr(v) rb_string_value_cstr(&(v))
#define SafeStringValue(v) StringValue(v)

/* arrays */
VALUE rb_ary_new(void);
VALUE rb_ary_new_capa(long capa);
#define rb_ary_new2 rb_ary_new_capa
VALUE rb_ary_new_from_values(long n, const VALUE *elts);
#define rb_ary_new4 rb_ary_new_from_values
VALUE rb_ary_new_from_args(long n, ...);
#define rb_ary_new3 rb_ary_new_from_args
VALUE rb_ary_push(VALUE a, VALUE v);
VALUE rb_ary_pop(VALUE a);
VALUE rb_ary_shift(VALUE a);
VALUE rb_ary_unshift(VALUE a, VALUE v);
VALUE rb_ary_entry(VALUE a, long i);
void rb_ary_store(VALUE a, long i, VALUE v);
VALUE rb_ary_dup(VALUE a);
VALUE rb_ary_clear(VALUE a);
VALUE rb_ary_join(VALUE a, VALUE sep);
long rb_kn_rarray_len(VALUE a);
VALUE *rb_kn_rarray_ptr(VALUE a);
#define RARRAY_LEN(a) rb_kn_rarray_len((VALUE)(a))
#define RARRAY_LENINT(a) ((int)RARRAY_LEN(a))
#define RARRAY_PTR(a) rb_kn_rarray_ptr((VALUE)(a))
#define RARRAY_CONST_PTR(a) ((const VALUE *)rb_kn_rarray_ptr((VALUE)(a)))
#define RARRAY_AREF(a, i) rb_ary_entry((a), (i))

/* hashes */
VALUE rb_hash_new(void);
VALUE rb_hash_aref(VALUE h, VALUE k);
VALUE rb_hash_lookup(VALUE h, VALUE k);
VALUE rb_hash_lookup2(VALUE h, VALUE k, VALUE def);
VALUE rb_hash_aset(VALUE h, VALUE k, VALUE v);
VALUE rb_hash_delete(VALUE h, VALUE k);
VALUE rb_hash_size(VALUE h);
VALUE rb_hash_keys(VALUE h);
enum st_retval { ST_CONTINUE = 0, ST_STOP = 1, ST_DELETE = 2, ST_CHECK = 3 };
void rb_hash_foreach(VALUE h, int (*func)(VALUE, VALUE, VALUE), VALUE arg);

/* objects & calls */
VALUE rb_obj_class(VALUE v);
VALUE rb_class_of(VALUE v);
#define CLASS_OF(v) rb_class_of((VALUE)(v))
const char *rb_obj_classname(VALUE v);
const char *rb_class2name(VALUE c);
VALUE rb_obj_is_kind_of(VALUE v, VALUE c);
VALUE rb_obj_is_instance_of(VALUE v, VALUE c);
int rb_respond_to(VALUE v, ID id);
VALUE rb_obj_freeze(VALUE v);
VALUE rb_obj_frozen_p(VALUE v);
void rb_check_frozen(VALUE v);
#define OBJ_FREEZE(v) rb_obj_freeze(v)
#define OBJ_FROZEN(v) RTEST(rb_obj_frozen_p(v))
VALUE rb_obj_alloc(VALUE klass);
VALUE rb_class_new_instance(int argc, const VALUE *argv, VALUE klass);
VALUE rb_funcall(VALUE recv, ID mid, int n, ...);
VALUE rb_funcallv(VALUE recv, ID mid, int argc, const VALUE *argv);
#define rb_funcall2 rb_funcallv
#define rb_funcallv_public rb_funcallv
VALUE rb_funcall_with_block(VALUE recv, ID mid, int argc, const VALUE *argv, VALUE block);
VALUE rb_yield(VALUE v);
VALUE rb_yield_values(int n, ...);
VALUE rb_yield_values2(int n, const VALUE *argv);
int rb_block_given_p(void);
VALUE rb_block_proc(void);
VALUE rb_ivar_get(VALUE obj, ID id);
VALUE rb_ivar_set(VALUE obj, ID id, VALUE v);
VALUE rb_iv_get(VALUE obj, const char *name);
VALUE rb_iv_set(VALUE obj, const char *name, VALUE v);
VALUE rb_attr_get(VALUE obj, ID id);
VALUE rb_gv_get(const char *name);
VALUE rb_gv_set(const char *name, VALUE v);
VALUE rb_const_get(VALUE klass, ID id);
int rb_const_defined(VALUE klass, ID id);
VALUE rb_path2class(const char *path);
VALUE rb_equal(VALUE a, VALUE b);
int rb_scan_args(int argc, const VALUE *argv, const char *fmt, ...);

/* exceptions */
NORETURN(void rb_raise(VALUE klass, const char *fmt, ...));
NORETURN(void rb_exc_raise(VALUE exc));
NORETURN(void rb_notimplement(void));
NORETURN(void rb_jump_tag(int state));
void rb_warn(const char *fmt, ...);
void rb_warning(const char *fmt, ...);
VALUE rb_errinfo(void);
void rb_set_errinfo(VALUE e);
VALUE rb_protect(VALUE (*func)(VALUE), VALUE arg, int *state);
VALUE rb_ensure(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*e_proc)(VALUE), VALUE data2);
VALUE rb_rescue(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2);
VALUE rb_rescue2(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2, ...);

/* data objects */
typedef void (*RUBY_DATA_FUNC)(void *);
#define RUBY_DEFAULT_FREE ((RUBY_DATA_FUNC)-1)
#define RUBY_NEVER_FREE ((RUBY_DATA_FUNC)0)
#define RUBY_TYPED_FREE_IMMEDIATELY 1
#define RUBY_TYPED_WB_PROTECTED 0
#define RUBY_TYPED_FROZEN_SHAREABLE 0
typedef struct rb_data_type_struct rb_data_type_t;
struct rb_data_type_struct {
  const char *wrap_struct_name;
  struct {
    RUBY_DATA_FUNC dmark;
    RUBY_DATA_FUNC dfree;
    size_t (*dsize)(const void *);
    RUBY_DATA_FUNC dcompact;
    void *reserved[1];
  } function;
  const rb_data_type_t *parent;
  void *data;
  VALUE flags;
};
VALUE rb_data_object_wrap(VALUE klass, void *ptr, RUBY_DATA_FUNC mark, RUBY_DATA_FUNC free);
VALUE rb_data_typed_object_wrap(VALUE klass, void *ptr, const rb_data_type_t *type);
VALUE rb_data_typed_object_zalloc(VALUE klass, size_t size, const rb_data_type_t *type);
void *rb_check_typeddata(VALUE obj, const rb_data_type_t *type);
void **rb_kn_data_ptr_ref(VALUE obj);
#define DATA_PTR(obj) (*rb_kn_data_ptr_ref((VALUE)(obj)))
#define Data_Wrap_Struct(klass, mark, free, sval) rb_data_object_wrap((klass), (void *)(sval), (RUBY_DATA_FUNC)(mark), (RUBY_DATA_FUNC)(free))
#define Data_Make_Struct(klass, type, mark, free, sval) \
  ((sval) = (type *)calloc(1, sizeof(type)), Data_Wrap_Struct((klass), (mark), (free), (sval)))
#define Data_Get_Struct(obj, type, sval) ((sval) = (type *)DATA_PTR(obj))
#define TypedData_Wrap_Struct(klass, data_type, sval) rb_data_typed_object_wrap((klass), (void *)(sval), (data_type))
#define TypedData_Make_Struct(klass, type, data_type, sval) \
  ((sval) = (type *)calloc(1, sizeof(type)), TypedData_Wrap_Struct((klass), (data_type), (sval)))
#define TypedData_Get_Struct(obj, type, data_type, sval) ((sval) = (type *)rb_check_typeddata((obj), (data_type)))
#define RTYPEDDATA_DATA(obj) DATA_PTR(obj)

/* memory */
#define ALLOC(type) ((type *)malloc(sizeof(type)))
#define ALLOC_N(type, n) ((type *)malloc(sizeof(type) * (n)))
#define ZALLOC(type) ((type *)calloc(1, sizeof(type)))
#define ZALLOC_N(type, n) ((type *)calloc((n), sizeof(type)))
#define REALLOC_N(var, type, n) ((var) = (type *)realloc((var), sizeof(type) * (n)))
#define ALLOCA_N(type, n) ((type *)__builtin_alloca(sizeof(type) * (n)))
#define xmalloc malloc
#define xcalloc calloc
#define xrealloc realloc
#define xfree free
#define MEMCPY(p1, p2, type, n) memcpy((p1), (p2), sizeof(type) * (n))
#define MEMZERO(p, type, n) memset((p), 0, sizeof(type) * (n))

/* GC */
void rb_gc_mark(VALUE v);
void rb_gc_mark_maybe(VALUE v);
void rb_gc_register_address(VALUE *addr);
void rb_global_variable(VALUE *addr);
void rb_gc_register_mark_object(VALUE v);
void rb_gc(void);
#define RB_GC_GUARD(v) (*(volatile VALUE *)&(v))

#ifdef __cplusplus
}
#endif
#endif
