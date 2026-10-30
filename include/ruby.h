/* Kenny's CRuby-compatible C API. Compile extensions with -std=gnu17 (C) or -std=gnu++17 (C++).
 * The other <ruby/...> headers an extension includes all lead here. */
#ifndef KENNY_RUBY_H
#define KENNY_RUBY_H 1

#ifdef RUBY_EXTCONF_H
#include RUBY_EXTCONF_H
#endif

#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <stdio.h>
#include <limits.h>
#include <inttypes.h>
#include <sys/types.h>
#include <unistd.h>
#include <alloca.h>
#include <time.h>
#include <sys/time.h>
#include <strings.h>
#ifndef __cplusplus
#include <stdbool.h>
#include <math.h>
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef uintptr_t VALUE;
typedef uintptr_t ID;
typedef intptr_t SIGNED_VALUE;
typedef uintptr_t st_data_t;
typedef st_data_t st_index_t;

/* the headers CRuby's ruby.h says it has (all of them lead to this file) */
#define HAVE_RUBY_DEFINES_H 1
#define HAVE_RUBY_ENCODING_H 1
#define HAVE_RUBY_INTERN_H 1
#define HAVE_RUBY_IO_H 1
#define HAVE_RUBY_MISSING_H 1
#define HAVE_RUBY_RE_H 1
#define HAVE_RUBY_RUBY_H 1
#define HAVE_RUBY_ST_H 1
#define HAVE_RUBY_THREAD_H 1
#define HAVE_RUBY_UTIL_H 1
#define HAVE_RUBY_VERSION_H 1

#define RUBY_EXTERN extern
/* old-style prototypes: `void f _((int));` (ruby/backward/2/stdarg.h) */
#ifndef _
#define _(args) args
#endif
#ifndef __
#define __(args) args
#endif
#ifdef __cplusplus
#define ANYARGS ...
#else
#define ANYARGS
#endif
#define RUBY_METHOD_FUNC(f) ((VALUE (*)(ANYARGS))(f))
#define RUBY_FUNC_EXPORTED __attribute__((visibility("default")))
#define RUBY_SYMBOL_EXPORT_BEGIN
#define RUBY_SYMBOL_EXPORT_END
#define NORETURN(x) __attribute__((noreturn)) x
#define DEPRECATED(x) x
#define PRINTF_ARGS(decl, a, b) decl
#define RB_LIKELY(x) __builtin_expect(!!(x), 1)
#define RB_UNLIKELY(x) __builtin_expect(!!(x), 0)
#ifndef LIKELY
#define LIKELY(x) RB_LIKELY(x)
#endif
#ifndef UNLIKELY
#define UNLIKELY(x) RB_UNLIKELY(x)
#endif
#define UNREACHABLE __builtin_unreachable()
#define UNREACHABLE_RETURN(v) __builtin_unreachable()
#define RUBY_API_VERSION_MAJOR 3
#define RUBY_API_VERSION_MINOR 4
#define RUBY_API_VERSION_TEENY 0
#define RUBY_API_VERSION_CODE (RUBY_API_VERSION_MAJOR * 10000 + RUBY_API_VERSION_MINOR * 100 + RUBY_API_VERSION_TEENY)
#define SIZEOF_INT 4
#define SIZEOF_LONG 8
#define SIZEOF_LONG_LONG 8
#define SIZEOF_VOIDP 8
#define SIZEOF_VALUE 8
#define SIZEOF_SIZE_T 8
#define HAVE_LONG_LONG 1
#define HAVE_RB_DATA_TYPE_T_FUNCTION 1
#define HAVE_RB_DATA_TYPE_T_PARENT 1
#define HAVE_TYPE_RB_DATA_TYPE_T 1
#define HAVE_RB_ARY_NEW_CAPA 1
#define HAVE_RB_STR_SET_LEN 1
#define HAVE_RB_HASH_NEW_CAPA 1
#define HAVE_RB_ENC_INTERNED_STR 1
#define HAVE_RB_PROC_CALL_WITH_BLOCK 1
#define HAVE_RB_GC_MARK_LOCATIONS 1
#define HAVE_RB_ERRINFO 1
#define HAVE_RB_BLOCK_CALL 1
#define HAVE_RB_ENSURE 1
/* rb_raise / rb_sprintf: "%"PRIsVALUE formats an object (to_s; "%+"PRIsVALUE: inspect) */
#define PRI_VALUE_PREFIX "l"
#define RUBY_PRI_VALUE_MARK "\v"
#define PRIsVALUE PRI_VALUE_PREFIX "i" RUBY_PRI_VALUE_MARK
#define PRIdVALUE PRIdPTR
#define PRIuVALUE PRIuPTR
#define PRIxVALUE PRIxPTR

/* immediates: same tagging as CRuby */
#define Qfalse ((VALUE)0x00)
#define Qnil ((VALUE)0x08)
#define Qtrue ((VALUE)0x14)
#define Qundef ((VALUE)0x34)
#define RUBY_Qfalse Qfalse
#define RUBY_Qnil Qnil
#define RUBY_Qtrue Qtrue
#define RUBY_Qundef Qundef

#define RTEST(v) ((((VALUE)(v)) & ~Qnil) != 0)
#define NIL_P(v) ((VALUE)(v) == Qnil)
#define RB_TEST(v) RTEST(v)
#define RB_NIL_P(v) NIL_P(v)
#define FIXNUM_P(v) ((((VALUE)(v)) & 1) == 1)
#define STATIC_SYM_P(v) ((((VALUE)(v)) & 0xff) == 0x0c)
#define SYMBOL_P(v) STATIC_SYM_P(v)
#define RB_SYMBOL_P(v) SYMBOL_P(v)
#define RB_FIXNUM_P(v) FIXNUM_P(v)
#define RB_INTEGER_TYPE_P(v) (FIXNUM_P(v) || RB_TYPE_P(v, T_BIGNUM))
/* flonums: Floats stored in the VALUE itself (low bits 0b10), as in 64-bit CRuby */
#define RUBY_FLONUM_MASK 0x03
#define RUBY_FLONUM_FLAG 0x02
#define FLONUM_P(v) ((((VALUE)(v)) & RUBY_FLONUM_MASK) == RUBY_FLONUM_FLAG)
#define RB_FLONUM_P(v) FLONUM_P(v)
#define USE_FLONUM 1
#define SPECIAL_CONST_P(v) (FIXNUM_P(v) || FLONUM_P(v) || SYMBOL_P(v) || (v) == Qnil || (v) == Qtrue || (v) == Qfalse)
#define RB_SPECIAL_CONST_P(v) SPECIAL_CONST_P(v)
#define IMMEDIATE_P(v) (FIXNUM_P(v) || FLONUM_P(v) || SYMBOL_P(v) || (v) == Qtrue)
#define INT2FIX(i) ((VALUE)((((SIGNED_VALUE)(i)) << 1) | 1))
#define LONG2FIX(i) INT2FIX(i)
#define RB_INT2FIX(i) INT2FIX(i)
#define FIX2LONG(v) ((long)(((SIGNED_VALUE)(v)) >> 1))
#define FIX2INT(v) ((int)FIX2LONG(v))
#define FIX2UINT(v) ((unsigned int)FIX2LONG(v))
#define FIX2ULONG(v) ((unsigned long)FIX2LONG(v))
#define RB_FIX2LONG(v) FIX2LONG(v)
#define FIXNUM_MAX (LONG_MAX >> 1)
#define RUBY_FIXNUM_MAX FIXNUM_MAX
#define RUBY_FIXNUM_MIN FIXNUM_MIN
#define FIXNUM_MIN (LONG_MIN >> 1)
#define POSFIXABLE(n) ((n) <= FIXNUM_MAX)
#define NEGFIXABLE(n) ((n) >= FIXNUM_MIN)
#define FIXABLE(n) (POSFIXABLE(n) && NEGFIXABLE(n))
#define RB_POSFIXABLE(n) POSFIXABLE(n)
#define RB_NEGFIXABLE(n) NEGFIXABLE(n)
#define RB_FIXABLE(n) FIXABLE(n)
#define CHR2FIX(c) INT2FIX((long)((c) & 0xff))

enum ruby_value_type {
  T_NONE = 0x00, T_OBJECT = 0x01, T_CLASS = 0x02, T_MODULE = 0x03, T_FLOAT = 0x04,
  T_STRING = 0x05, T_REGEXP = 0x06, T_ARRAY = 0x07, T_HASH = 0x08, T_STRUCT = 0x09,
  T_BIGNUM = 0x0a, T_FILE = 0x0b, T_DATA = 0x0c, T_MATCH = 0x0d, T_COMPLEX = 0x0e,
  T_RATIONAL = 0x0f, T_NIL = 0x11, T_TRUE = 0x12, T_FALSE = 0x13, T_SYMBOL = 0x14,
  T_FIXNUM = 0x15, T_UNDEF = 0x16, T_IMEMO = 0x1a, T_NODE = 0x1b, T_ICLASS = 0x1c, T_ZOMBIE = 0x1d,
  T_MOVED = 0x1e, T_MASK = 0x1f
};
#define RUBY_T_NONE T_NONE
#define RUBY_T_OBJECT T_OBJECT
#define RUBY_T_CLASS T_CLASS
#define RUBY_T_MODULE T_MODULE
#define RUBY_T_FLOAT T_FLOAT
#define RUBY_T_STRING T_STRING
#define RUBY_T_REGEXP T_REGEXP
#define RUBY_T_ARRAY T_ARRAY
#define RUBY_T_HASH T_HASH
#define RUBY_T_STRUCT T_STRUCT
#define RUBY_T_BIGNUM T_BIGNUM
#define RUBY_T_FILE T_FILE
#define RUBY_T_DATA T_DATA
#define RUBY_T_MATCH T_MATCH
#define RUBY_T_COMPLEX T_COMPLEX
#define RUBY_T_RATIONAL T_RATIONAL
#define RUBY_T_NIL T_NIL
#define RUBY_T_TRUE T_TRUE
#define RUBY_T_FALSE T_FALSE
#define RUBY_T_SYMBOL T_SYMBOL
#define RUBY_T_FIXNUM T_FIXNUM
#define RUBY_T_UNDEF T_UNDEF

int rb_type(VALUE v);
#define TYPE(v) rb_type((VALUE)(v))
#define RB_TYPE(v) rb_type((VALUE)(v))
#define BUILTIN_TYPE(v) rb_type((VALUE)(v))
#define RB_BUILTIN_TYPE(v) rb_type((VALUE)(v))
#define RB_TYPE_P(v, t) (rb_type((VALUE)(v)) == (t))
#define RB_FLOAT_TYPE_P(v) (FLONUM_P(v) || RB_TYPE_P(v, T_FLOAT))
void rb_check_type(VALUE v, int t);
#define Check_Type(v, t) rb_check_type((VALUE)(v), (t))

/* class / exception globals */
RUBY_EXTERN VALUE rb_cObject, rb_cBasicObject, rb_cModule, rb_cClass, rb_mKernel, rb_mComparable,
    rb_mEnumerable, rb_cString, rb_cSymbol, rb_cArray, rb_cHash, rb_cInteger, rb_cFloat, rb_cNumeric,
    rb_cRange, rb_cProc, rb_cNilClass, rb_cTrueClass, rb_cFalseClass, rb_cStruct, rb_cTime, rb_cIO,
    rb_cRational, rb_cComplex, rb_cEncoding, rb_cRegexp, rb_cMatch, rb_cMethod, rb_cUnboundMethod,
    rb_cThread, rb_cRandom, rb_cDir, rb_cFile, rb_cBinding, rb_cEnumerator, rb_mMath, rb_mProcess,
    rb_mGC, rb_mErrno, rb_cSet, rb_cData;
RUBY_EXTERN VALUE rb_eException, rb_eStandardError, rb_eRuntimeError, rb_eArgError, rb_eTypeError,
    rb_eNameError, rb_eNoMethodError, rb_eIndexError, rb_eKeyError, rb_eRangeError, rb_eZeroDivError,
    rb_eNotImpError, rb_eIOError, rb_eStopIteration, rb_eFrozenError, rb_eNoMemError, rb_eScriptError,
    rb_eFloatDomainError, rb_eEncodingError, rb_eLocalJumpError, rb_eSecurityError, rb_eEOFError,
    rb_eThreadError, rb_eSystemCallError, rb_eRegexpError, rb_eSyntaxError, rb_eLoadError,
    rb_eInterrupt, rb_eSignal, rb_eFatal, rb_eSystemExit, rb_eMathDomainError,
    rb_eEncCompatError, rb_eNoMatchingPatternError, rb_eNoMatchingPatternKeyError, rb_eSysStackError;
#define rb_cFixnum rb_cInteger
#define rb_cBignum rb_cInteger

/* definitions */
VALUE rb_define_module(const char *name);
VALUE rb_define_module_under(VALUE outer, const char *name);
VALUE rb_define_module_id_under(VALUE outer, ID name);
VALUE rb_define_class(const char *name, VALUE super);
VALUE rb_define_class_under(VALUE outer, const char *name, VALUE super);
VALUE rb_define_class_id_under(VALUE outer, ID name, VALUE super);
VALUE rb_class_new(VALUE super);
VALUE rb_module_new(void);
void rb_define_method(VALUE klass, const char *name, VALUE (*func)(ANYARGS), int argc);
void rb_define_private_method(VALUE klass, const char *name, VALUE (*func)(ANYARGS), int argc);
void rb_define_protected_method(VALUE klass, const char *name, VALUE (*func)(ANYARGS), int argc);
void rb_define_singleton_method(VALUE obj, const char *name, VALUE (*func)(ANYARGS), int argc);
void rb_define_module_function(VALUE mod, const char *name, VALUE (*func)(ANYARGS), int argc);
void rb_define_global_function(const char *name, VALUE (*func)(ANYARGS), int argc);
void rb_define_method_id(VALUE klass, ID name, VALUE (*func)(ANYARGS), int argc);
void rb_undef_method(VALUE klass, const char *name);
void rb_undef(VALUE klass, ID id);
void rb_define_alias(VALUE klass, const char *new_name, const char *old_name);
void rb_alias(VALUE klass, ID new_name, ID old_name);
void rb_define_attr(VALUE klass, const char *name, int read, int write);
void rb_attr(VALUE klass, ID name, int read, int write, int ex);
void rb_define_const(VALUE klass, const char *name, VALUE val);
void rb_define_global_const(const char *name, VALUE val);
void rb_const_set(VALUE klass, ID id, VALUE val);
void rb_define_alloc_func(VALUE klass, VALUE (*func)(VALUE));
typedef VALUE (*rb_alloc_func_t)(VALUE klass);
/* the class's allocator, NULL when it has none of its own */
rb_alloc_func_t rb_get_alloc_func(VALUE klass);
/* anonymous ones (CRuby: named by `name` once assigned to a constant) */
VALUE rb_define_class_id(ID name, VALUE super);
VALUE rb_define_module_id(ID name);
void rb_undef_alloc_func(VALUE klass);
void rb_include_module(VALUE klass, VALUE module);
void rb_extend_object(VALUE obj, VALUE module);
void rb_prepend_module(VALUE klass, VALUE module);
void rb_ext_ractor_safe(bool flag);
#define RB_EXT_RACTOR_SAFE(f) rb_ext_ractor_safe(f)
#define HAVE_RB_EXT_RACTOR_SAFE 1

/* symbols */
ID rb_intern(const char *name);
ID rb_intern2(const char *name, long len);
ID rb_intern_str(VALUE str);
#define rb_intern_const(s) rb_intern(s)
#define RUBY_CONST_ID(var, str) do { (var) = rb_intern(str); } while (0)
#define CONST_ID(var, str) RUBY_CONST_ID(var, str)
VALUE rb_id2sym(ID id);
ID rb_sym2id(VALUE sym);
const char *rb_id2name(ID id);
VALUE rb_id2str(ID id);
VALUE rb_sym2str(VALUE sym);
VALUE rb_to_symbol(VALUE name);
VALUE rb_check_symbol(volatile VALUE *namep);
ID rb_to_id(VALUE v);
ID rb_check_id(volatile VALUE *namep);
int rb_is_const_id(ID id);
int rb_is_instance_id(ID id);
#define ID2SYM(id) rb_id2sym(id)
#define SYM2ID(s) rb_sym2id(s)
#define RB_ID2SYM(id) rb_id2sym(id)
#define RB_SYM2ID(s) rb_sym2id(s)

/* numbers */
VALUE rb_int2inum(intptr_t n);
VALUE rb_uint2inum(uintptr_t n);
VALUE rb_ll2inum(long long n);
VALUE rb_ull2inum(unsigned long long n);
VALUE rb_int2big(intptr_t n);
VALUE rb_uint2big(uintptr_t n);
VALUE rb_float_new(double f);
VALUE rb_float_new_in_heap(double f);
long rb_num2long(VALUE v);
unsigned long rb_num2ulong(VALUE v);
long long rb_num2ll(VALUE v);
unsigned long long rb_num2ull(VALUE v);
long rb_num2int(VALUE v);
unsigned long rb_num2uint(VALUE v);
long rb_fix2int(VALUE v);
short rb_num2short(VALUE v);
unsigned short rb_num2ushort(VALUE v);
double rb_num2dbl(VALUE v);
char rb_num2chr(VALUE v);
#define INT2NUM(i) rb_int2inum((intptr_t)(i))
#define LONG2NUM(i) rb_int2inum((intptr_t)(i))
#define UINT2NUM(i) rb_uint2inum((uintptr_t)(i))
#define ULONG2NUM(i) rb_uint2inum((uintptr_t)(i))
#define LL2NUM(i) rb_ll2inum(i)
#define ULL2NUM(i) rb_ull2inum(i)
#define SIZET2NUM(i) rb_uint2inum((uintptr_t)(i))
#define SSIZET2NUM(i) rb_int2inum((intptr_t)(i))
#define OFFT2NUM(i) rb_ll2inum((long long)(i))
#define PIDT2NUM(i) rb_int2inum((intptr_t)(i))
#define RB_INT2NUM INT2NUM
#define RB_LONG2NUM LONG2NUM
#define RB_UINT2NUM UINT2NUM
#define RB_ULONG2NUM ULONG2NUM
#define NUM2LONG(v) (FIXNUM_P(v) ? FIX2LONG(v) : rb_num2long(v))
#define NUM2INT(v) ((int)rb_num2int(v))
#define NUM2UINT(v) ((unsigned int)rb_num2uint(v))
#define NUM2ULONG(v) rb_num2ulong(v)
#define NUM2LL(v) rb_num2ll(v)
#define NUM2ULL(v) rb_num2ull(v)
#define NUM2SIZET(v) ((size_t)rb_num2ulong(v))
#define NUM2SSIZET(v) ((ssize_t)rb_num2long(v))
#define NUM2OFFT(v) ((off_t)rb_num2ll(v))
#define NUM2SHORT(v) rb_num2short(v)
#define NUM2USHORT(v) rb_num2ushort(v)
#define NUM2CHR(v) rb_num2chr(v)
#define NUM2DBL(v) rb_num2dbl(v)
#define NUM2PIDT(v) ((pid_t)rb_num2long(v))
#define RB_NUM2LONG NUM2LONG
#define RB_NUM2INT NUM2INT
#define RB_NUM2UINT NUM2UINT
#define RB_NUM2ULONG NUM2ULONG
#define DBL2NUM(f) rb_float_new(f)
#define RFLOAT_VALUE(v) rb_float_value(v)
VALUE rb_Integer(VALUE v);
VALUE rb_Float(VALUE v);
VALUE rb_to_int(VALUE v);
VALUE rb_to_float(VALUE v);
VALUE rb_check_to_integer(VALUE v, const char *method);
VALUE rb_check_to_int(VALUE v);
VALUE rb_cstr2inum(const char *s, int base);
VALUE rb_str2inum(VALUE s, int base);
VALUE rb_cstr_to_inum(const char *s, int base, int badcheck);
VALUE rb_str_to_inum(VALUE str, int base, int badcheck);
/* Integer <-> words (CRuby's bignum.c) */
#define INTEGER_PACK_MSWORD_FIRST 0x01
#define INTEGER_PACK_LSWORD_FIRST 0x02
#define INTEGER_PACK_MSBYTE_FIRST 0x10
#define INTEGER_PACK_LSBYTE_FIRST 0x20
#define INTEGER_PACK_NATIVE 0x40
#define INTEGER_PACK_2COMP 0x80
#define INTEGER_PACK_FORCE_BIGNUM 0x100
#define INTEGER_PACK_NEGATIVE 0x200
#define INTEGER_PACK_FORCE_GENERIC_IMPLEMENTATION 0x400
#define INTEGER_PACK_LITTLE_ENDIAN (INTEGER_PACK_LSWORD_FIRST | INTEGER_PACK_LSBYTE_FIRST)
#define INTEGER_PACK_BIG_ENDIAN (INTEGER_PACK_MSWORD_FIRST | INTEGER_PACK_MSBYTE_FIRST)
int rb_integer_pack(VALUE val, void *words, size_t numwords, size_t wordsize, size_t nails, int flags);
VALUE rb_integer_unpack(const void *words, size_t numwords, size_t wordsize, size_t nails, int flags);
VALUE rb_str_encode_ospath(VALUE path);
int rb_reserved_fd_p(int fd);
#define RB_RESERVED_FD_P(fd) rb_reserved_fd_p(fd)
#define rb_isspace(c) ISSPACE(c)
#define rb_isdigit(c) ISDIGIT(c)
#define rb_isalpha(c) ISALPHA(c)
#define rb_isalnum(c) ISALNUM(c)
#define rb_isupper(c) ISUPPER(c)
#define rb_islower(c) ISLOWER(c)
#define rb_isxdigit(c) ISXDIGIT(c)
#define rb_isprint(c) ISPRINT(c)
double rb_cstr_to_dbl(const char *s, int badcheck);
double rb_str_to_dbl(VALUE s, int badcheck);
VALUE rb_dbl2big(double d);
VALUE rb_rational_new(VALUE num, VALUE den);
VALUE rb_Rational(VALUE num, VALUE den);
VALUE rb_complex_new(VALUE re, VALUE im);
#define rb_rational_new1(n) rb_rational_new((n), INT2FIX(1))
#define rb_Rational1(n) rb_Rational((n), INT2FIX(1))
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
/* bignums */
int rb_big_sign(VALUE b);
#define RBIGNUM_SIGN(b) rb_big_sign(b)
#define RBIGNUM_POSITIVE_P(b) (RBIGNUM_SIGN(b) != 0)
#define RBIGNUM_NEGATIVE_P(b) (RBIGNUM_SIGN(b) == 0)
#define BIGNUM_SIGN(b) RBIGNUM_SIGN(b)
#define BIGNUM_POSITIVE_P(b) RBIGNUM_POSITIVE_P(b)
#define BIGNUM_NEGATIVE_P(b) RBIGNUM_NEGATIVE_P(b)
long rb_big2long(VALUE b);
unsigned long rb_big2ulong(VALUE b);
long long rb_big2ll(VALUE b);
unsigned long long rb_big2ull(VALUE b);
double rb_big2dbl(VALUE b);
VALUE rb_big2str(VALUE b, int base);
VALUE rb_big_cmp(VALUE a, VALUE b);
VALUE rb_big_eq(VALUE a, VALUE b);
VALUE rb_big_plus(VALUE a, VALUE b);
VALUE rb_big_minus(VALUE a, VALUE b);
VALUE rb_big_mul(VALUE a, VALUE b);
VALUE rb_big_norm(VALUE b);
size_t rb_absint_size(VALUE v, int *nlz_bits);
int rb_absint_singlebit_p(VALUE v);

/* encodings */
typedef struct kn_encoding {
  const char *name;
  int index;
  int min_len;
  int max_len;
  int ascii_compat;
  int dummy;
} rb_encoding;
typedef rb_encoding OnigEncodingType;
typedef const rb_encoding *OnigEncoding;
ID rb_intern3(const char *name, long len, rb_encoding *enc);
#define ENCODING_SHIFT 22
#define ENCODING_MASK (((VALUE)0xff) << ENCODING_SHIFT)
#define ENCODING_MAXNAMELEN 42
#define rb_enc_name(e) ((e)->name)
#define rb_enc_mbminlen(e) ((e)->min_len)
#define rb_enc_mbmaxlen(e) ((e)->max_len)
#define rb_enc_asciicompat(e) ((e)->ascii_compat)
#define rb_enc_dummy_p(e) ((e)->dummy)
#define rb_enc_to_index(e) ((e) ? (e)->index : 0)
#define ENC_TO_ENCINDEX(e) rb_enc_to_index(e)
rb_encoding *rb_utf8_encoding(void);
rb_encoding *rb_ascii8bit_encoding(void);
rb_encoding *rb_usascii_encoding(void);
rb_encoding *rb_locale_encoding(void);
rb_encoding *rb_filesystem_encoding(void);
rb_encoding *rb_default_external_encoding(void);
rb_encoding *rb_default_internal_encoding(void);
VALUE rb_enc_default_external(void);
VALUE rb_enc_default_internal(void);
int rb_utf8_encindex(void);
int rb_ascii8bit_encindex(void);
int rb_usascii_encindex(void);
int rb_locale_encindex(void);
int rb_filesystem_encindex(void);
rb_encoding *rb_enc_from_index(int idx);
int rb_enc_find_index(const char *name);
rb_encoding *rb_enc_find(const char *name);
rb_encoding *rb_to_encoding(VALUE enc);
int rb_to_encoding_index(VALUE enc);
VALUE rb_enc_from_encoding(rb_encoding *enc);
int rb_enc_get_index(VALUE obj);
rb_encoding *rb_enc_get(VALUE obj);
void rb_enc_set_index(VALUE obj, int idx);
VALUE rb_enc_associate_index(VALUE obj, int idx);
VALUE rb_enc_associate(VALUE obj, rb_encoding *enc);
void rb_enc_copy(VALUE dst, VALUE src);
rb_encoding *rb_enc_compatible(VALUE a, VALUE b);
rb_encoding *rb_enc_check(VALUE a, VALUE b);
int rb_enc_str_coderange(VALUE str);
int rb_enc_str_asciionly_p(VALUE str);
long rb_enc_strlen(const char *p, const char *e, rb_encoding *enc);
int rb_enc_mbclen(const char *p, const char *e, rb_encoding *enc);
int rb_enc_precise_mbclen(const char *p, const char *e, rb_encoding *enc);
int rb_enc_codelen(int code, rb_encoding *enc);
unsigned int rb_enc_codepoint_len(const char *p, const char *e, int *len, rb_encoding *enc);
#define rb_enc_codepoint(p, e, enc) rb_enc_codepoint_len((p), (e), 0, (enc))
int rb_enc_uint_chr(unsigned int code, rb_encoding *enc);
VALUE rb_enc_str_new(const char *p, long len, rb_encoding *enc);
VALUE rb_enc_str_new_cstr(const char *p, rb_encoding *enc);
#define rb_enc_str_new_static rb_enc_str_new
VALUE rb_enc_interned_str(const char *p, long len, rb_encoding *enc);
VALUE rb_enc_interned_str_cstr(const char *p, rb_encoding *enc);
VALUE rb_enc_str_buf_cat(VALUE s, const char *p, long len, rb_encoding *enc);
VALUE rb_str_export_to_enc(VALUE str, rb_encoding *enc);
VALUE rb_str_conv_enc(VALUE str, rb_encoding *from, rb_encoding *to);
VALUE rb_str_encode(VALUE str, VALUE to, int ecflags, VALUE ecopts);
#define rb_enc_asciicompat_p rb_enc_asciicompat
#define ENCODING_INLINE_MAX 127
#define ENCODING_GET(obj) rb_enc_get_index(obj)
#define ENCODING_GET_INLINED(obj) rb_enc_get_index(obj)
#define ENCODING_SET(obj, i) rb_enc_set_index((obj), (i))
#define ENCODING_SET_INLINED(obj, i) rb_enc_set_index((obj), (i))
#define ENCODING_IS_ASCII8BIT(obj) (rb_enc_get_index(obj) == 0)
#define RB_ENCODING_GET(obj) ENCODING_GET(obj)
#define RB_ENCODING_SET(obj, i) ENCODING_SET(obj, i)
#define RUBY_ENC_CODERANGE_UNKNOWN 0
#define RUBY_ENC_CODERANGE_7BIT ((int)(1 << 20))
#define RUBY_ENC_CODERANGE_VALID ((int)(1 << 21))
#define RUBY_ENC_CODERANGE_BROKEN ((int)(RUBY_ENC_CODERANGE_7BIT | RUBY_ENC_CODERANGE_VALID))
#define ENC_CODERANGE_UNKNOWN RUBY_ENC_CODERANGE_UNKNOWN
#define ENC_CODERANGE_7BIT RUBY_ENC_CODERANGE_7BIT
#define ENC_CODERANGE_VALID RUBY_ENC_CODERANGE_VALID
#define ENC_CODERANGE_BROKEN RUBY_ENC_CODERANGE_BROKEN
#define ENC_CODERANGE(obj) rb_enc_str_coderange(obj)
#define ENC_CODERANGE_ASCIIONLY(obj) (ENC_CODERANGE(obj) == ENC_CODERANGE_7BIT)
#define ENC_CODERANGE_CLEAR(obj) ((void)0)
#define ENC_CODERANGE_SET(obj, cr) ((void)0)
#define rb_enc_isascii(c, enc) ((unsigned)(c) < 128)
#define rb_enc_isspace(c, enc) ((c) == ' ' || ((c) >= '\t' && (c) <= '\r'))
#define rb_enc_isdigit(c, enc) ((unsigned)((c) - '0') < 10)
#define rb_enc_isalpha(c, enc) ((unsigned)(((c) | 0x20) - 'a') < 26)
#define rb_enc_isalnum(c, enc) (rb_enc_isalpha(c, enc) || rb_enc_isdigit(c, enc))
#define rb_enc_isprint(c, enc) ((c) >= 0x20 && (c) < 0x7f)
#define rb_enc_left_char_head(s, p, e, enc) (p)
#define MBCLEN_CHARFOUND_P(ret) (0 < (ret))
#define MBCLEN_CHARFOUND_LEN(ret) (ret)
#define MBCLEN_INVALID_P(ret) ((ret) == -1)
#define MBCLEN_NEEDMORE_P(ret) ((ret) < -1)
#define MBCLEN_NEEDMORE_LEN(ret) (-1 - (ret))
#define ECONV_INVALID_REPLACE 0x0002
#define ECONV_UNDEF_REPLACE 0x0020
#define ECONV_XML_TEXT_DECORATOR 0x4000

/* strings */
VALUE rb_str_new(const char *p, long len);
VALUE rb_str_new_cstr(const char *p);
VALUE rb_utf8_str_new(const char *p, long len);
VALUE rb_utf8_str_new_cstr(const char *p);
VALUE rb_usascii_str_new(const char *p, long len);
VALUE rb_usascii_str_new_cstr(const char *p);
VALUE rb_external_str_new(const char *p, long len);
VALUE rb_external_str_new_cstr(const char *p);
VALUE rb_locale_str_new(const char *p, long len);
VALUE rb_locale_str_new_cstr(const char *p);
VALUE rb_filesystem_str_new(const char *p, long len);
VALUE rb_filesystem_str_new_cstr(const char *p);
#define rb_str_new2 rb_str_new_cstr
#define rb_str_new3 rb_str_new_shared
#define rb_str_new4 rb_str_new_frozen
#define rb_str_new_static rb_str_new
#define rb_usascii_str_new2 rb_usascii_str_new_cstr
#define rb_usascii_str_new_static rb_usascii_str_new
#define rb_utf8_str_new_static rb_utf8_str_new
#define rb_str_new_literal(s) rb_str_new((s), (long)(sizeof(s) - 1))
#define rb_usascii_str_new_literal(s) rb_usascii_str_new((s), (long)(sizeof(s) - 1))
#define rb_utf8_str_new_literal(s) rb_utf8_str_new((s), (long)(sizeof(s) - 1))
#define rb_tainted_str_new rb_str_new
#define rb_tainted_str_new2 rb_str_new_cstr
#define rb_tainted_str_new_cstr rb_str_new_cstr
VALUE rb_str_new_shared(VALUE s);
VALUE rb_str_new_frozen(VALUE s);
VALUE rb_str_new_with_class(VALUE obj, const char *p, long len);
VALUE rb_interned_str(const char *p, long len);
VALUE rb_interned_str_cstr(const char *p);
VALUE rb_fstring(VALUE s);
VALUE rb_str_cat(VALUE s, const char *p, long len);
VALUE rb_str_cat_cstr(VALUE s, const char *p);
#define rb_str_cat2 rb_str_cat_cstr
#define rb_str_buf_cat rb_str_cat
#define rb_str_buf_cat2 rb_str_cat_cstr
#define rb_str_buf_cat_ascii rb_str_cat_cstr
#define rb_str_cat_literal(s, lit) rb_str_cat((s), (lit), (long)(sizeof(lit) - 1))
VALUE rb_str_append(VALUE s, VALUE o);
VALUE rb_str_buf_append(VALUE s, VALUE o);
VALUE rb_str_concat(VALUE s, VALUE o);
VALUE rb_str_plus(VALUE a, VALUE b);
VALUE rb_str_times(VALUE s, VALUE n);
VALUE rb_str_dup(VALUE s);
VALUE rb_str_resurrect(VALUE s);
VALUE rb_str_resize(VALUE s, long len);
void rb_str_set_len(VALUE s, long len);
VALUE rb_str_buf_new(long capa);
VALUE rb_str_buf_new_cstr(const char *p);
#define rb_str_buf_new2 rb_str_buf_new_cstr
size_t rb_str_capacity(VALUE s);
void rb_str_modify(VALUE s);
void rb_str_modify_expand(VALUE s, long extra);
VALUE rb_str_freeze(VALUE s);
VALUE rb_str_replace(VALUE s, VALUE o);
VALUE rb_str_substr(VALUE s, long beg, long len);
VALUE rb_str_subseq(VALUE s, long beg, long len);
VALUE rb_str_equal(VALUE a, VALUE b);
int rb_str_cmp(VALUE a, VALUE b);
st_index_t rb_str_hash(VALUE s);
/* hashing helpers (CRuby's st_hash_* family; the values differ from CRuby's, as its are
   seeded per process anyway) */
static inline st_index_t st_hash_uint(st_index_t h, st_index_t i) {
    h ^= i + 0x9e3779b97f4a7c15ULL + (h << 6) + (h >> 2);
    return h;
}
static inline st_index_t st_hash_uint32(st_index_t h, uint32_t i) { return st_hash_uint(h, (st_index_t)i); }
static inline st_index_t st_hash_end(st_index_t h) {
    h ^= h >> 33; h *= 0xff51afd7ed558ccdULL; h ^= h >> 33; h *= 0xc4ceb9fe1a85ec53ULL; h ^= h >> 33;
    return h;
}
static inline st_index_t st_hash_start(st_index_t h) { return h ^ 0x2545f4914f6cdd1dULL; }
static inline st_index_t st_hash(const void *ptr, size_t len, st_index_t h) {
    const unsigned char *p = (const unsigned char *)ptr;
    st_index_t x = h ^ 0xcbf29ce484222325ULL;
    for (size_t k = 0; k < len; k++) { x ^= p[k]; x *= 0x100000001b3ULL; }
    return st_hash_end(x);
}
static inline st_index_t rb_memhash(const void *ptr, long len) { return st_hash(ptr, (size_t)len, 0); }
static inline st_index_t rb_hash_start(st_index_t h) { return st_hash_start(h); }
#define rb_hash_uint32(h, i) st_hash_uint32((h), (i))
#define rb_hash_uint(h, i) st_hash_uint((h), (i))
#define rb_hash_end(h) st_hash_end(h)
VALUE rb_str_length(VALUE s);
long rb_str_strlen(VALUE s);
VALUE rb_str_intern(VALUE s);
VALUE rb_str_to_str(VALUE s);
VALUE rb_str_inspect(VALUE s);
VALUE rb_str_dump(VALUE s);
VALUE rb_str_split(VALUE s, const char *sep);
VALUE rb_str_scrub(VALUE s, VALUE repl);
VALUE rb_str_succ(VALUE s);
void rb_str_free(VALUE s);
VALUE rb_str_locktmp(VALUE s);
VALUE rb_str_unlocktmp(VALUE s);
VALUE rb_check_string_type(VALUE v);
VALUE rb_obj_as_string(VALUE v);
VALUE rb_any_to_s(VALUE v);
VALUE rb_String(VALUE v);
VALUE rb_inspect(VALUE v);
VALUE rb_sprintf(const char *fmt, ...);
VALUE rb_vsprintf(const char *fmt, va_list ap);
VALUE rb_str_catf(VALUE s, const char *fmt, ...);
VALUE rb_str_vcatf(VALUE s, const char *fmt, va_list ap);
char *rb_kn_rstring_ptr(VALUE s);
long rb_kn_rstring_len(VALUE s);
#define RSTRING_PTR(s) rb_kn_rstring_ptr((VALUE)(s))
#define RSTRING_LEN(s) rb_kn_rstring_len((VALUE)(s))
#define RSTRING_END(s) (RSTRING_PTR(s) + RSTRING_LEN(s))
#define RSTRING_LENINT(s) ((int)RSTRING_LEN(s))
#define RSTRING_GETMEM(s, ptrvar, lenvar) ((ptrvar) = RSTRING_PTR(s), (lenvar) = RSTRING_LEN(s))
#define RSTRING(s) (s)
#define RSTRING_EMBED_LEN_MAX 23
VALUE rb_string_value(volatile VALUE *ptr);
char *rb_string_value_ptr(volatile VALUE *ptr);
char *rb_string_value_cstr(volatile VALUE *ptr);
#define StringValue(v) rb_string_value(&(v))
#define StringValuePtr(v) rb_string_value_ptr(&(v))
#define StringValueCStr(v) rb_string_value_cstr(&(v))
#define SafeStringValue(v) StringValue(v)
#define FilePathValue(v) StringValue(v)
#define ExportStringValue(v) StringValue(v)
#define rb_str_export rb_str_dup
#define rb_str_export_locale rb_str_dup

/* arrays */
VALUE rb_ary_new(void);
VALUE rb_ary_new_capa(long capa);
#define rb_ary_new2 rb_ary_new_capa
VALUE rb_ary_new_from_values(long n, const VALUE *elts);
#define rb_ary_new4 rb_ary_new_from_values
VALUE rb_ary_new_from_args(long n, ...);
#define rb_ary_new3 rb_ary_new_from_args
#define rb_ary_tmp_new rb_ary_new_capa
#define rb_ary_hidden_new rb_ary_new_capa
VALUE rb_assoc_new(VALUE a, VALUE b);
VALUE rb_ary_push(VALUE a, VALUE v);
VALUE rb_ary_cat(VALUE a, const VALUE *p, long n);
VALUE rb_ary_pop(VALUE a);
VALUE rb_ary_shift(VALUE a);
VALUE rb_ary_unshift(VALUE a, VALUE v);
VALUE rb_ary_entry(VALUE a, long i);
VALUE rb_ary_aref1(VALUE a, VALUE i);
void rb_ary_store(VALUE a, long i, VALUE v);
VALUE rb_ary_dup(VALUE a);
VALUE rb_ary_clear(VALUE a);
VALUE rb_ary_join(VALUE a, VALUE sep);
VALUE rb_ary_concat(VALUE a, VALUE b);
VALUE rb_ary_plus(VALUE a, VALUE b);
VALUE rb_ary_includes(VALUE a, VALUE v);
VALUE rb_ary_delete(VALUE a, VALUE v);
VALUE rb_ary_delete_at(VALUE a, long i);
VALUE rb_ary_reverse(VALUE a);
VALUE rb_ary_sort(VALUE a);
VALUE rb_ary_sort_bang(VALUE a);
VALUE rb_ary_subseq(VALUE a, long beg, long len);
VALUE rb_ary_freeze(VALUE a);
VALUE rb_ary_resize(VALUE a, long len);
VALUE rb_ary_to_ary(VALUE v);
VALUE rb_ary_to_s(VALUE a);
VALUE rb_ary_replace(VALUE a, VALUE b);
VALUE rb_ary_assoc(VALUE a, VALUE k);
VALUE rb_ary_cmp(VALUE a, VALUE b);
VALUE rb_check_array_type(VALUE v);
VALUE rb_Array(VALUE v);
long rb_kn_rarray_len(VALUE a);
VALUE *rb_kn_rarray_ptr(VALUE a);
#define RARRAY_LEN(a) rb_kn_rarray_len((VALUE)(a))
#define RARRAY_LENINT(a) ((int)RARRAY_LEN(a))
#define RARRAY_PTR(a) rb_kn_rarray_ptr((VALUE)(a))
#define RARRAY_CONST_PTR(a) ((const VALUE *)rb_kn_rarray_ptr((VALUE)(a)))
#define RARRAY_CONST_PTR_TRANSIENT(a) RARRAY_CONST_PTR(a)
#define RARRAY_AREF(a, i) rb_ary_entry((a), (i))
#define RARRAY_ASET(a, i, v) rb_ary_store((a), (i), (v))
#define RARRAY_PTR_USE(a, ptr_name, expr) do { VALUE *ptr_name = RARRAY_PTR(a); expr; } while (0)
#define RARRAY_EMBED_LEN_MAX 3
#define RARRAY(a) (a)

/* hashes */
VALUE rb_hash_new(void);
VALUE rb_hash_new_capa(long capa);
VALUE rb_hash_aref(VALUE h, VALUE k);
VALUE rb_hash_lookup(VALUE h, VALUE k);
VALUE rb_hash_lookup2(VALUE h, VALUE k, VALUE def);
VALUE rb_hash_fetch(VALUE h, VALUE k);
VALUE rb_hash_aset(VALUE h, VALUE k, VALUE v);
VALUE rb_hash_delete(VALUE h, VALUE k);
VALUE rb_hash_delete_if(VALUE h);
VALUE rb_hash_clear(VALUE h);
VALUE rb_hash_size(VALUE h);
size_t rb_hash_size_num(VALUE h);
VALUE rb_hash_keys(VALUE h);
VALUE rb_hash_values(VALUE h);
VALUE rb_hash_dup(VALUE h);
VALUE rb_hash_freeze(VALUE h);
VALUE rb_hash_set_ifnone(VALUE h, VALUE ifnone);
void rb_hash_bulk_insert(long n, const VALUE *argv, VALUE h);
VALUE rb_hash(VALUE obj);
VALUE rb_check_hash_type(VALUE v);
VALUE rb_Hash(VALUE v);
#define RHASH_SIZE(h) rb_hash_size_num(h)
#define RHASH_EMPTY_P(h) (RHASH_SIZE(h) == 0)
#define RHASH_IFNONE(h) rb_hash_ifnone(h)
VALUE rb_hash_ifnone(VALUE h);
#define HAVE_RB_HASH_BULK_INSERT 1
enum st_retval { ST_CONTINUE = 0, ST_STOP = 1, ST_DELETE = 2, ST_CHECK = 3, ST_REPLACE = 4 };
void rb_hash_foreach(VALUE h, int (*func)(VALUE, VALUE, VALUE), VALUE arg);
#ifdef __cplusplus
}
static inline void rb_hash_foreach(VALUE h, int (*func)(...), VALUE arg) { rb_hash_foreach(h, (int (*)(VALUE, VALUE, VALUE))func, arg); }
extern "C" {
#endif

/* structs */
VALUE rb_struct_define(const char *name, ...);
VALUE rb_struct_define_under(VALUE outer, const char *name, ...);
VALUE rb_struct_new(VALUE klass, ...);
VALUE rb_struct_alloc(VALUE klass, VALUE values);
VALUE rb_struct_alloc_noinit(VALUE klass);
VALUE rb_struct_aref(VALUE st, VALUE idx);
VALUE rb_struct_aset(VALUE st, VALUE idx, VALUE v);
VALUE rb_struct_getmember(VALUE st, ID id);
VALUE rb_struct_members(VALUE st);
VALUE rb_struct_s_members(VALUE klass);
VALUE rb_struct_size(VALUE st);
#define RSTRUCT_LEN(st) NUM2LONG(rb_struct_size(st))
#define RSTRUCT_GET(st, i) rb_struct_aref((st), INT2NUM(i))
#define RSTRUCT_SET(st, i, v) rb_struct_aset((st), INT2NUM(i), (v))

/* ranges */
VALUE rb_range_new(VALUE beg, VALUE end, int excl);
int rb_range_values(VALUE range, VALUE *begp, VALUE *endp, int *exclp);
/* a Range's begin and length within a sequence of `len`: Qtrue, Qfalse for a non-Range,
   Qnil out of range (err 1: RangeError instead) */
VALUE rb_range_beg_len(VALUE range, long *begp, long *lenp, long len, int err);
VALUE rb_reg_new_str(VALUE src, int opts);
VALUE rb_reg_new(const char *src, long len, int opts);
VALUE rb_obj_encoding(VALUE obj);

/* objects & calls */
VALUE rb_obj_class(VALUE v);
VALUE rb_class_of(VALUE v);
#define CLASS_OF(v) rb_class_of((VALUE)(v))
#define RBASIC_CLASS(v) rb_class_of((VALUE)(v))
VALUE rb_singleton_class(VALUE v);
const char *rb_obj_classname(VALUE v);
const char *rb_class2name(VALUE c);
VALUE rb_class_name(VALUE c);
VALUE rb_class_path(VALUE c);
VALUE rb_mod_name(VALUE c);
VALUE rb_class_superclass(VALUE c);
VALUE rb_class_get_superclass(VALUE c);
VALUE rb_class_real(VALUE c);
VALUE rb_class_inherited_p(VALUE mod, VALUE arg);
VALUE rb_mod_ancestors(VALUE mod);
VALUE rb_mod_include_p(VALUE mod, VALUE mod2);
VALUE rb_obj_is_kind_of(VALUE v, VALUE c);
VALUE rb_obj_is_instance_of(VALUE v, VALUE c);
VALUE rb_obj_is_proc(VALUE v);
VALUE rb_obj_is_method(VALUE v);
int rb_respond_to(VALUE v, ID id);
int rb_obj_respond_to(VALUE v, ID id, int priv);
VALUE rb_obj_freeze(VALUE v);
VALUE rb_obj_frozen_p(VALUE v);
void rb_check_frozen(VALUE v);
#define rb_check_frozen_inline rb_check_frozen
#define OBJ_FREEZE(v) rb_obj_freeze(v)
#define RB_OBJ_FREEZE(v) rb_obj_freeze(v)
#define OBJ_FROZEN(v) RTEST(rb_obj_frozen_p(v))
#define RB_OBJ_FROZEN(v) OBJ_FROZEN(v)
#define OBJ_FROZEN_RAW(v) OBJ_FROZEN(v)
#define RB_OBJ_FROZEN_RAW(v) OBJ_FROZEN(v)
#define OBJ_TAINT(v) ((void)0)
#define OBJ_TAINTED(v) 0
#define OBJ_UNTRUST(v) ((void)0)
#define OBJ_INFECT(a, b) ((void)0)
#define RB_OBJ_TAINT(v) ((void)0)
VALUE rb_obj_dup(VALUE v);
VALUE rb_obj_clone(VALUE v);
VALUE rb_obj_id(VALUE v);
VALUE rb_obj_hide(VALUE v);
VALUE rb_obj_reveal(VALUE v, VALUE klass);
VALUE rb_obj_alloc(VALUE klass);
void rb_obj_call_init(VALUE obj, int argc, const VALUE *argv);
void rb_obj_call_init_kw(VALUE obj, int argc, const VALUE *argv, int kw_splat);
VALUE rb_obj_instance_variables(VALUE obj);
VALUE rb_class_new_instance(int argc, const VALUE *argv, VALUE klass);
VALUE rb_class_new_instance_kw(int argc, const VALUE *argv, VALUE klass, int kw_splat);
VALUE rb_funcall(VALUE recv, ID mid, int n, ...);
VALUE rb_funcallv(VALUE recv, ID mid, int argc, const VALUE *argv);
VALUE rb_funcallv_kw(VALUE recv, ID mid, int argc, const VALUE *argv, int kw_splat);
VALUE rb_funcallv_public(VALUE recv, ID mid, int argc, const VALUE *argv);
VALUE rb_funcallv_public_kw(VALUE recv, ID mid, int argc, const VALUE *argv, int kw_splat);
#define rb_funcall2 rb_funcallv
#define rb_funcall3 rb_funcallv_public
VALUE rb_funcall_with_block(VALUE recv, ID mid, int argc, const VALUE *argv, VALUE block);
VALUE rb_funcall_passing_block(VALUE recv, ID mid, int argc, const VALUE *argv);
VALUE rb_funcall_passing_block_kw(VALUE recv, ID mid, int argc, const VALUE *argv, int kw_splat);
VALUE rb_check_funcall(VALUE recv, ID mid, int argc, const VALUE *argv);
VALUE rb_apply(VALUE recv, ID mid, VALUE args);
VALUE rb_eval_string(const char *src);
VALUE rb_eval_string_protect(const char *src, int *state);
VALUE rb_require(const char *feature);
VALUE rb_f_require(VALUE self, VALUE feature);
VALUE rb_yield(VALUE v);
VALUE rb_yield_values(int n, ...);
VALUE rb_yield_values2(int n, const VALUE *argv);
VALUE rb_yield_values_kw(int n, const VALUE *argv, int kw_splat);
VALUE rb_yield_splat(VALUE args);
VALUE rb_yield_block(VALUE yielded_arg, VALUE callback_arg, int argc, const VALUE *argv, VALUE blockarg);
int rb_block_given_p(void);
void rb_need_block(void);
VALUE rb_block_proc(void);
VALUE rb_block_lambda(void);
int rb_keyword_given_p(void);
#define RB_NO_KEYWORDS 0
#define RB_PASS_KEYWORDS 1
#define RB_PASS_CALLED_KEYWORDS rb_keyword_given_p()
#define RB_BLOCK_CALL_FUNC_ARGLIST(yielded_arg, callback_arg) \
  VALUE yielded_arg, VALUE callback_arg, int argc, const VALUE *argv, VALUE blockarg
typedef VALUE rb_block_call_func(RB_BLOCK_CALL_FUNC_ARGLIST(yielded_arg, callback_arg));
typedef rb_block_call_func *rb_block_call_func_t;
VALUE rb_block_call(VALUE obj, ID mid, int argc, const VALUE *argv, rb_block_call_func_t bl_proc, VALUE data2);
VALUE rb_block_call_kw(VALUE obj, ID mid, int argc, const VALUE *argv, rb_block_call_func_t bl_proc, VALUE data2, int kw_splat);
VALUE rb_iterate(VALUE (*it_proc)(VALUE), VALUE data1, rb_block_call_func_t bl_proc, VALUE data2);
VALUE rb_each(VALUE obj);
VALUE rb_proc_new(rb_block_call_func_t func, VALUE val);
VALUE rb_proc_call(VALUE proc, VALUE args);
VALUE rb_proc_call_kw(VALUE proc, VALUE args, int kw_splat);
VALUE rb_proc_call_with_block(VALUE proc, int argc, const VALUE *argv, VALUE passed_proc);
int rb_proc_arity(VALUE proc);
VALUE rb_proc_lambda_p(VALUE proc);
VALUE rb_method_call(int argc, const VALUE *argv, VALUE method);
ID rb_frame_this_func(void);
ID rb_frame_callee(void);
VALUE rb_current_receiver(void);
VALUE rb_enumeratorize(VALUE obj, VALUE meth, int argc, const VALUE *argv);
VALUE rb_enumeratorize_with_size(VALUE obj, VALUE meth, int argc, const VALUE *argv, VALUE (*size_fn)(VALUE, VALUE, VALUE));
#define SIZED_ENUMERATOR(obj, argc, argv, size_fn) \
  rb_enumeratorize_with_size((obj), ID2SYM(rb_frame_this_func()), (argc), (argv), (size_fn))
#define RETURN_SIZED_ENUMERATOR(obj, argc, argv, size_fn) \
  do { if (!rb_block_given_p()) return SIZED_ENUMERATOR((obj), (argc), (argv), (size_fn)); } while (0)
#define RETURN_ENUMERATOR(obj, argc, argv) RETURN_SIZED_ENUMERATOR((obj), (argc), (argv), 0)
#define RETURN_SIZED_ENUMERATOR_KW(obj, argc, argv, size_fn, kw) RETURN_SIZED_ENUMERATOR((obj), (argc), (argv), (size_fn))
#define RETURN_ENUMERATOR_KW(obj, argc, argv, kw) RETURN_ENUMERATOR((obj), (argc), (argv))
VALUE rb_ivar_get(VALUE obj, ID id);
VALUE rb_ivar_set(VALUE obj, ID id, VALUE v);
VALUE rb_ivar_defined(VALUE obj, ID id);
VALUE rb_iv_get(VALUE obj, const char *name);
VALUE rb_iv_set(VALUE obj, const char *name, VALUE v);
VALUE rb_attr_get(VALUE obj, ID id);
VALUE rb_cvar_get(VALUE klass, ID id);
void rb_cvar_set(VALUE klass, ID id, VALUE v);
VALUE rb_cvar_defined(VALUE klass, ID id);
VALUE rb_cv_get(VALUE klass, const char *name);
void rb_cv_set(VALUE klass, const char *name, VALUE v);
void rb_define_class_variable(VALUE klass, const char *name, VALUE v);
VALUE rb_gv_get(const char *name);
VALUE rb_gv_set(const char *name, VALUE v);
void rb_define_variable(const char *name, VALUE *var);
void rb_define_readonly_variable(const char *name, const VALUE *var);
VALUE rb_const_get(VALUE klass, ID id);
VALUE rb_const_get_at(VALUE klass, ID id);
VALUE rb_const_get_from(VALUE klass, ID id);
int rb_const_defined(VALUE klass, ID id);
int rb_const_defined_at(VALUE klass, ID id);
int rb_const_defined_from(VALUE klass, ID id);
VALUE rb_const_remove(VALUE klass, ID id);
VALUE rb_path2class(const char *path);
VALUE rb_path_to_class(VALUE path);
VALUE rb_equal(VALUE a, VALUE b);
int rb_eql(VALUE a, VALUE b);
VALUE rb_obj_equal(VALUE a, VALUE b);
VALUE rb_convert_type(VALUE v, int type, const char *tname, const char *method);
VALUE rb_check_convert_type(VALUE v, int type, const char *tname, const char *method);
int rb_scan_args(int argc, const VALUE *argv, const char *fmt, ...);
int rb_scan_args_kw(int kw_splat, int argc, const VALUE *argv, const char *fmt, ...);
int rb_get_kwargs(VALUE hash, const ID *table, int required, int optional, VALUE *values);
static inline void rb_check_arity(int argc, int min, int max);
void rb_error_arity(int argc, int min, int max);
#define UNLIMITED_ARGUMENTS (-1)
#define rb_exc_new2 rb_exc_new_cstr
#define rb_exc_new3 rb_exc_new_str

/* exceptions */
NORETURN(void rb_raise(VALUE klass, const char *fmt, ...));
NORETURN(void rb_exc_raise(VALUE exc));
NORETURN(void rb_notimplement(void));
NORETURN(void rb_jump_tag(int state));
NORETURN(void rb_bug(const char *fmt, ...));
NORETURN(void rb_fatal(const char *fmt, ...));
NORETURN(void rb_sys_fail(const char *msg));
NORETURN(void rb_sys_fail_str(VALUE msg));
NORETURN(void rb_syserr_fail(int err, const char *msg));
NORETURN(void rb_syserr_fail_str(int err, VALUE msg));
NORETURN(void rb_enc_raise(rb_encoding *enc, VALUE klass, const char *fmt, ...));
NORETURN(void rb_frozen_error_raise(VALUE obj, const char *fmt, ...));
NORETURN(void rb_error_frozen(const char *what));
NORETURN(void rb_error_frozen_object(VALUE obj));
NORETURN(void rb_name_error(ID name, const char *fmt, ...));
NORETURN(void rb_invalid_str(const char *str, const char *type));
NORETURN(void rb_memerror(void));
NORETURN(void rb_throw(const char *tag, VALUE val));
NORETURN(void rb_throw_obj(VALUE tag, VALUE val));
NORETURN(void rb_exit(int status));
VALUE rb_exc_new(VALUE klass, const char *p, long len);
VALUE rb_exc_new_cstr(VALUE klass, const char *p);
VALUE rb_exc_new_str(VALUE klass, VALUE msg);
VALUE rb_syserr_new(int err, const char *msg);
void rb_warn(const char *fmt, ...);
void rb_warning(const char *fmt, ...);
void rb_category_warn(int category, const char *fmt, ...);
#define RB_WARN_CATEGORY_DEPRECATED 1
#define RB_WARN_CATEGORY_EXPERIMENTAL 2
VALUE rb_errinfo(void);
void rb_set_errinfo(VALUE e);
VALUE rb_protect(VALUE (*func)(VALUE), VALUE arg, int *state);
VALUE rb_ensure(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*e_proc)(VALUE), VALUE data2);
VALUE rb_rescue(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2);
VALUE rb_rescue2(VALUE (*b_proc)(VALUE), VALUE data1, VALUE (*r_proc)(VALUE, VALUE), VALUE data2, ...);
#define ruby_verbose (*rb_ruby_verbose_ptr())
#define ruby_debug (*rb_ruby_debug_ptr())
VALUE *rb_ruby_verbose_ptr(void);
VALUE *rb_ruby_debug_ptr(void);
static inline void rb_check_arity(int argc, int min, int max) {
  if (argc < min || (max != UNLIMITED_ARGUMENTS && argc > max)) rb_error_arity(argc, min, max);
}

/* data objects */
typedef void (*RUBY_DATA_FUNC)(void *);
#define RUBY_DEFAULT_FREE ((RUBY_DATA_FUNC)-1)
#define RUBY_NEVER_FREE ((RUBY_DATA_FUNC)0)
#define RUBY_TYPED_DEFAULT_FREE RUBY_DEFAULT_FREE
#define RUBY_TYPED_NEVER_FREE RUBY_NEVER_FREE
#define RUBY_TYPED_FREE_IMMEDIATELY 1
#define RUBY_TYPED_WB_PROTECTED 0
#define RUBY_TYPED_FROZEN_SHAREABLE 0
#define RUBY_TYPED_DECL_MARKING 0
#define RUBY_FL_WB_PROTECTED 0
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
VALUE rb_data_object_zalloc(VALUE klass, size_t size, RUBY_DATA_FUNC mark, RUBY_DATA_FUNC free);
VALUE rb_data_typed_object_wrap(VALUE klass, void *ptr, const rb_data_type_t *type);
VALUE rb_data_typed_object_zalloc(VALUE klass, size_t size, const rb_data_type_t *type);
void *rb_check_typeddata(VALUE obj, const rb_data_type_t *type);
int rb_typeddata_is_kind_of(VALUE obj, const rb_data_type_t *type);
int rb_typeddata_inherited_p(const rb_data_type_t *child, const rb_data_type_t *parent);
const rb_data_type_t *rb_kn_typeddata_type(VALUE obj);
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
#define RTYPEDDATA_GET_DATA(obj) DATA_PTR(obj)
#define RTYPEDDATA_P(obj) (rb_kn_typeddata_type(obj) != NULL)
#define RTYPEDDATA_TYPE(obj) rb_kn_typeddata_type(obj)
#define RTYPEDDATA(obj) (obj)
#define RDATA(obj) (obj)
#define rb_data_object_get DATA_PTR

/* memory */
void *ruby_xmalloc(size_t n);
void *ruby_xmalloc2(size_t n, size_t size);
void *ruby_xcalloc(size_t n, size_t size);
void *ruby_xrealloc(void *p, size_t n);
void *ruby_xrealloc2(void *p, size_t n, size_t size);
void ruby_xfree(void *p);
#define xmalloc ruby_xmalloc
#define xmalloc2 ruby_xmalloc2
#define xcalloc ruby_xcalloc
#define xrealloc ruby_xrealloc
#define xrealloc2 ruby_xrealloc2
#define xfree ruby_xfree
#define ALLOC(type) ((type *)ruby_xmalloc(sizeof(type)))
#define ALLOC_N(type, n) ((type *)ruby_xmalloc2((n), sizeof(type)))
#define RB_ALLOC(type) ALLOC(type)
#define RB_ALLOC_N(type, n) ALLOC_N(type, n)
#define ZALLOC(type) ((type *)ruby_xcalloc(1, sizeof(type)))
#define ZALLOC_N(type, n) ((type *)ruby_xcalloc((n), sizeof(type)))
#define REALLOC_N(var, type, n) ((var) = (type *)ruby_xrealloc2((void *)(var), (n), sizeof(type)))
#define RB_REALLOC_N(var, type, n) REALLOC_N(var, type, n)
#define ALLOCA_N(type, n) ((type *)__builtin_alloca(sizeof(type) * (n)))
#define ALLOCV(v, n) ((v) = 0, __builtin_alloca(n))
#define ALLOCV_N(type, v, n) ((v) = 0, (type *)__builtin_alloca(sizeof(type) * (n)))
#define ALLOCV_END(v) ((void)(v))
#define MEMCPY(p1, p2, type, n) memcpy((p1), (p2), sizeof(type) * (size_t)(n))
#define MEMMOVE(p1, p2, type, n) memmove((p1), (p2), sizeof(type) * (size_t)(n))
#define MEMZERO(p, type, n) memset((p), 0, sizeof(type) * (size_t)(n))
#define MEMCMP(p1, p2, type, n) memcmp((p1), (p2), sizeof(type) * (size_t)(n))
char *ruby_strdup(const char *s);
int ruby_snprintf(char *buf, size_t n, const char *fmt, ...);
int ruby_vsnprintf(char *buf, size_t n, const char *fmt, va_list ap);
#define ISSPACE(c) ((c) == ' ' || ((c) >= '\t' && (c) <= '\r'))
#define ISDIGIT(c) ((unsigned)((c) - '0') < 10)
#define ISALPHA(c) ((unsigned)(((c) | 0x20) - 'a') < 26)
#define ISALNUM(c) (ISALPHA(c) || ISDIGIT(c))
#define ISUPPER(c) ((unsigned)((c) - 'A') < 26)
#define ISLOWER(c) ((unsigned)((c) - 'a') < 26)
#define ISPRINT(c) ((c) >= 0x20 && (c) < 0x7f)
#define ISXDIGIT(c) (ISDIGIT(c) || (unsigned)(((c) | 0x20) - 'a') < 6)
#define ISASCII(c) ((unsigned)(c) < 128)
#define TOUPPER(c) (ISLOWER(c) ? (c) - 32 : (c))
#define TOLOWER(c) (ISUPPER(c) ? (c) + 32 : (c))
#define STRCASECMP(a, b) strcasecmp((a), (b))
#define STRNCASECMP(a, b, n) strncasecmp((a), (b), (n))

/* GC: objects handed to C stay put while C code runs, and objects a mark function marks
 * (rb_gc_mark) never move, so pointers from RSTRING_PTR / DATA_PTR stay valid. */
void rb_gc_mark(VALUE v);
void rb_gc_mark_maybe(VALUE v);
void rb_gc_mark_movable(VALUE v);
void rb_gc_mark_locations(const VALUE *start, const VALUE *end);
VALUE rb_gc_location(VALUE v);
void rb_gc_register_address(VALUE *addr);
void rb_gc_unregister_address(VALUE *addr);
void rb_global_variable(VALUE *addr);
void rb_gc_register_mark_object(VALUE v);
void rb_gc(void);
void rb_gc_start(void);
VALUE rb_gc_enable(void);
VALUE rb_gc_disable(void);
void rb_gc_adjust_memory_usage(ssize_t diff);
void rb_gc_writebarrier(VALUE a, VALUE b);
void rb_gc_writebarrier_unprotect(VALUE obj);
size_t rb_gc_count(void);
int rb_during_gc(void);
#define RB_GC_GUARD(v) (*(volatile VALUE *)&(v))
#define RB_OBJ_WRITE(a, slot, b) (*(VALUE *)(slot) = (VALUE)(b), (VALUE)(a))
#define RB_OBJ_WRITTEN(a, oldv, b) ((void)(oldv), (void)(b), (VALUE)(a))
#define OBJ_WRITE(a, slot, b) RB_OBJ_WRITE(a, slot, b)
#define OBJ_WRITTEN(a, oldv, b) RB_OBJ_WRITTEN(a, oldv, b)

/* threads (Kenny runs C extension code like any native method; no GVL to release) */
void *rb_thread_call_without_gvl(void *(*func)(void *), void *data1, void (*ubf)(void *), void *data2);
int ruby_thread_has_gvl_p(void);
void *rb_thread_call_without_gvl2(void *(*func)(void *), void *data1, void (*ubf)(void *), void *data2);
void *rb_thread_call_with_gvl(void *(*func)(void *), void *data1);
void *rb_nogvl(void *(*func)(void *), void *data1, void (*ubf)(void *), void *data2, int flags);
#define RUBY_UBF_IO ((void (*)(void *))(-1))
#define RUBY_UBF_PROCESS ((void (*)(void *))(-1))
VALUE rb_thread_current(void);
VALUE rb_thread_main(void);
void rb_thread_schedule(void);
void rb_thread_check_ints(void);
int rb_thread_interrupted(VALUE th);
VALUE rb_thread_local_aref(VALUE th, ID id);
VALUE rb_thread_local_aset(VALUE th, ID id, VALUE v);
VALUE rb_mutex_new(void);
VALUE rb_mutex_lock(VALUE m);
VALUE rb_mutex_unlock(VALUE m);
VALUE rb_mutex_trylock(VALUE m);
VALUE rb_mutex_locked_p(VALUE m);
VALUE rb_mutex_synchronize(VALUE m, VALUE (*func)(VALUE), VALUE arg);

/* time */
VALUE rb_time_new(time_t sec, long usec);
VALUE rb_time_nano_new(time_t sec, long nsec);
struct timeval rb_time_timeval(VALUE time);
struct timespec rb_time_timespec(VALUE time);
/* offset: seconds east of UTC, INT_MAX for local time, INT_MAX-1 for UTC */
VALUE rb_time_timespec_new(const struct timespec *ts, int offset);
int rb_obj_method_arity(VALUE obj, ID mid);
int rb_mod_method_arity(VALUE mod, ID mid);
/* whether klass has the method (ex: public and protected ones only) */
int rb_method_boundp(VALUE klass, ID mid, int ex);
VALUE rb_call_super(int argc, const VALUE *argv);
VALUE rb_obj_method(VALUE obj, VALUE name);
VALUE rb_sym_to_s(VALUE sym);
void rb_copy_generic_ivar(VALUE clone, VALUE obj);
/* waiting on an IO (rb_io_wait: the events ready, or false on timeout) */
enum rb_io_event { RUBY_IO_READABLE = 1, RUBY_IO_PRIORITY = 2, RUBY_IO_WRITABLE = 4 };
VALUE rb_io_wait(VALUE io, VALUE events, VALUE timeout);
void rb_ivar_foreach(VALUE obj, int (*func)(ID key, VALUE val, st_data_t arg), st_data_t arg);
st_index_t rb_ivar_count(VALUE obj);
VALUE rb_check_symbol_cstr(const char *ptr, long len, rb_encoding *enc);
#define rb_array_len(a) RARRAY_LEN(a)

#ifdef __cplusplus
}
/* C++: method functions of any signature, as CRuby's ruby/backward/cxxanyargs.hpp allows */
template <typename R, typename... A> static inline void rb_define_method(VALUE k, const char *n, R (*f)(A...), int c) { rb_define_method(k, n, (VALUE (*)(...))f, c); }
template <typename R, typename... A> static inline void rb_define_private_method(VALUE k, const char *n, R (*f)(A...), int c) { rb_define_private_method(k, n, (VALUE (*)(...))f, c); }
template <typename R, typename... A> static inline void rb_define_protected_method(VALUE k, const char *n, R (*f)(A...), int c) { rb_define_protected_method(k, n, (VALUE (*)(...))f, c); }
template <typename R, typename... A> static inline void rb_define_singleton_method(VALUE k, const char *n, R (*f)(A...), int c) { rb_define_singleton_method(k, n, (VALUE (*)(...))f, c); }
template <typename R, typename... A> static inline void rb_define_module_function(VALUE k, const char *n, R (*f)(A...), int c) { rb_define_module_function(k, n, (VALUE (*)(...))f, c); }
template <typename R, typename... A> static inline void rb_define_global_function(const char *n, R (*f)(A...), int c) { rb_define_global_function(n, (VALUE (*)(...))f, c); }
#endif
#endif
