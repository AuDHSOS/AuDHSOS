/* SPDX-License-Identifier: AGPL-3.0-only
** Copyright (C) 2026 Manuel Baesler and contributors
**
** The recorded oracle: a program that links the C library and answers
** what it answers, so that a test can compare against SQLite itself
** without SQLite being there when the test runs.
**
** Built and run by `sh tools/sqlite-fixtures.sh`. One argument names what
** to answer; the cases arrive on stdin, one per line, and one line of
** answer comes back per case.
**
**   fp-corpus   writes the cases, so that what is compared is fixed and
**               the file is regenerated rather than edited.
**   fp          a double as sixteen hex digits of its IEEE 754 bits,
**               answered as three tab-separated renderings of it:
**               "%!.4g", "%!.17g" (what a value converts to) and
**               "%!.20g" (the most digits the flag allows).
**   num-corpus  writes the cases for the reader of numbers.
**   num         a piece of text as "x" and its bytes in hex, answered as
**               what sqlite3AtoF and sqlite3Atoi64 make of it: the code
**               each returns, the double, and the integer.
**   expr-corpus writes the cases for the reader of expressions.
**   expr        one SQL expression, answered as its type and its value
**               quoted, or as the message SQLite refused it with.
*/
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "sqlite3.h"

/* Two routines the amalgamation keeps to itself. The build defines
** SQLITE_PRIVATE to nothing so that they can be asked directly. */
extern int sqlite3AtoF(const char *, double *);
extern int sqlite3Atoi64(const char *, sqlite3_int64 *, int, unsigned char);

/* A double from sixteen hex digits, and the text SQLite prints it as. */
static int fp_case(const char *line) {
  sqlite3_uint64 bits = 0;
  double value;
  char *text;
  int at;
  for (at = 0; at < 16; at++) {
    char c = line[at];
    int digit;
    if (c >= '0' && c <= '9') {
      digit = c - '0';
    } else if (c >= 'a' && c <= 'f') {
      digit = c - 'a' + 10;
    } else if (c >= 'A' && c <= 'F') {
      digit = c - 'A' + 10;
    } else {
      return 1;
    }
    bits = (bits << 4) | (sqlite3_uint64)digit;
  }
  memcpy(&value, &bits, 8);
  text = sqlite3_mprintf("%!.4g\t%!.17g\t%!.20g", value, value, value);
  if (text == 0) return 1;
  printf("%s\n", text);
  sqlite3_free(text);
  return 0;
}

/* One case of the corpus. */
static void fp_emit(double value) {
  sqlite3_uint64 bits;
  memcpy(&bits, &value, 8);
  printf("%016llX\n", (unsigned long long)bits);
}

/* A case from its bits, for the patterns no decimal literal writes. */
static void fp_emit_bits(sqlite3_uint64 bits) {
  printf("%016llX\n", (unsigned long long)bits);
}

/* The cases: the values a reader would pick by hand, a sweep of the
** exponent range, and a deterministic run of arbitrary bit patterns. */
static void fp_corpus(void) {
  sqlite3_uint64 state = 0x9E3779B97F4A7C15ULL;
  int i;
  double one = 1.0, zero = 0.0;

  fp_emit(0.0);
  fp_emit(-0.0);
  fp_emit(one / zero);
  fp_emit(-one / zero);
  fp_emit(zero / zero);
  fp_emit_bits(0x0000000000000001ULL); /* the smallest subnormal */
  fp_emit_bits(0x000FFFFFFFFFFFFFULL); /* the largest subnormal */
  fp_emit_bits(0x0010000000000000ULL); /* the smallest normal */
  fp_emit_bits(0x7FEFFFFFFFFFFFFFULL); /* the largest finite */
  fp_emit_bits(0x8000000000000001ULL);
  fp_emit(49.47);  /* the value the shortening rule is written for */
  fp_emit(0.1);
  fp_emit(0.1 + 0.2);
  fp_emit(1.0 / 3.0);
  fp_emit(2.0 / 3.0);
  fp_emit(9007199254740992.0);
  fp_emit(9223372036854775808.0);

  /* Every power of ten the format reaches, and two neighbours of each. */
  for (i = -320; i <= 308; i++) {
    char spec[32];
    double value;
    sqlite3_snprintf(sizeof(spec), spec, "1e%d", i);
    value = strtod(spec, 0);
    fp_emit(value);
    sqlite3_snprintf(sizeof(spec), spec, "9.9e%d", i);
    fp_emit(strtod(spec, 0));
    sqlite3_snprintf(sizeof(spec), spec, "1.0000000000000002e%d", i);
    fp_emit(strtod(spec, 0));
  }

  /* Quotients, which is where runs of nines and of zeros come from. */
  for (i = 1; i <= 400; i++) {
    double n = (double)i;
    fp_emit(n);
    fp_emit(n / 7.0);
    fp_emit(n / 3.0);
    fp_emit(n / 1000.0);
    fp_emit(n * 1.1);
    fp_emit(n * 1e10);
    fp_emit(n * 1e-10);
    fp_emit(-n / 9.0);
  }

  /* Powers of two, across the whole exponent range. */
  for (i = -1074; i <= 1023; i += 7) {
    double value = 1.0;
    int k = i;
    while (k > 0) { value *= 2.0; k--; }
    while (k < 0) { value /= 2.0; k++; }
    fp_emit(value);
  }

  /* Arbitrary bit patterns, from a fixed seed so the file does not move. */
  for (i = 0; i < 3000; i++) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    fp_emit_bits(state);
  }
}

/* The bytes of one case, from the hex after the leading "x". */
static int num_bytes(const char *line, char *out, int *pLen) {
  int at = 1, len = 0;
  while (line[at] != 0) {
    int hi, lo;
    const char *digits = "0123456789ABCDEF";
    const char *p = strchr(digits, line[at]);
    if (p == 0 || line[at + 1] == 0) return 1;
    hi = (int)(p - digits);
    p = strchr(digits, line[at + 1]);
    if (p == 0) return 1;
    lo = (int)(p - digits);
    if (len >= 1020) return 1;
    out[len++] = (char)((hi << 4) | lo);
    at += 2;
  }
  out[len] = 0;
  *pLen = len;
  return 0;
}

/* What the two readers make of one case. */
static int num_case(const char *line) {
  char bytes[1024];
  int len = 0;
  double real = 0.0;
  sqlite3_int64 whole = 0;
  int rc_real, rc_whole;
  char *text;
  if (num_bytes(line, bytes, &len) != 0) return 1;
  rc_real = sqlite3AtoF(bytes, &real);
  rc_whole = sqlite3Atoi64(bytes, &whole, len, SQLITE_UTF8);
  text = sqlite3_mprintf("%d\t%!.17g\t%d\t%lld", rc_real, real, rc_whole,
                         (long long)whole);
  if (text == 0) return 1;
  printf("%s\n", text);
  sqlite3_free(text);
  return 0;
}

/* One case of the corpus, as "x" and the bytes in hex. */
static void num_emit(const char *text) {
  int at;
  printf("x");
  for (at = 0; text[at] != 0; at++) {
    printf("%02X", (unsigned char)text[at]);
  }
  printf("\n");
}

/* The cases: every rule of the two readers, and the edges of each. */
static void num_corpus(void) {
  static const char *aCase[] = {
    "", " ", "  ", "\t", "\n", "\r", "\f", "\v", "0", "1", "-1", "+1",
    "-0", "+0", "00", "007", "0007.5", "1.", "1.0", ".5", "-.5", "+.5", ".",
    "-.", "+.", ".e5", "1.5", "1e3", "1E3", "1e+3", "1e-3", "1e", "1E", "1e+",
    "1e-", "1.2e3", "1.2E-3", "1e0", "0e0", "0.0", "000.000", "0e100",
    "0e-100", "abc", "e5", "E5", "-abc", "12abc", "12 abc", " 12 ", "  12  ",
    "12\t", "\t12", "- 1", "+ 1", "--1", "++1", "-+1", "0x10", "0X10", "1_0",
    "1,000", "nan", "NaN", "inf", "Inf", "-inf", "infinity", "Infinity",
    "9223372036854775806", "9223372036854775807", "9223372036854775808",
    "9223372036854775809", "-9223372036854775807", "-9223372036854775808",
    "-9223372036854775809", "18446744073709551615", "18446744073709551616",
    "18446744073709551617", "0009223372036854775807", "922337203685477580",
    "9223372036854775800", "9999999999999999999", "1000000000000000000",
    "99999999999999999999", "999999999999999999999999999999",
    "12345678901234567890123456789012345678901234567890",
    "1.7976931348623157e308", "1.7976931348623159e308", "1e308", "1e309",
    "1e-308", "1e-323", "1e-324", "1e-400", "1e1000", "1e-1000", "1e99999",
    "1e-99999", "1e10000", "1e10001", "-1e10000", "1e999999", "1e-999999",
    "1e12345678901234567890", "1.5e0000000000000000000000099999999", "3.141592653589793",
    "3.14159265358979323846264338327950288", "2.2250738585072014e-308",
    "4.9406564584124654e-324", "0.1", "0.2", "0.3", "49.47", "0.000001",
    "1000000", "123456789012345678901234567890.12345678901234567890",
    "0.00000000000000000000000000000000000001", "1.5e", "1.5e+", "1.5ee5",
    "1.5e5.5", ".0e0", "0.e0", "1.e5", "5e-0", "5e+0", "5e00000000003",
    "5e000000000000000000000003", "  -12.5e-3  ", "\t\n\r 42 \f\v",
    "42\0abc", "1\0001", "-", "+", "  -  ", "e", "E", ".5e3", "-0.0",
    "-0e0", "0.0000000000000000000000000000000000000000000000001"
  };
  int i;
  for (i = 0; i < (int)(sizeof(aCase) / sizeof(aCase[0])); i++) {
    num_emit(aCase[i]);
  }
  /* Digit strings of every length, which is where the mantissa fills up
  ** and where nineteen digits meet the limit of an integer. */
  for (i = 1; i <= 25; i++) {
    char buf[64];
    int j;
    for (j = 0; j < i; j++) buf[j] = (char)('1' + (j % 9));
    buf[i] = 0;
    num_emit(buf);
    buf[i] = '.';
    buf[i + 1] = '5';
    buf[i + 2] = 0;
    num_emit(buf);
  }
  /* A point at every place of a long digit string. */
  for (i = 0; i <= 22; i++) {
    char buf[64];
    int j, k = 0;
    for (j = 0; j < 22; j++) {
      if (j == i) buf[k++] = '.';
      buf[k++] = (char)('0' + ((j * 7) % 10));
    }
    buf[k] = 0;
    num_emit(buf);
  }
}

/* The database every expression is put to. It holds no tables: what is
** asked is what an expression alone answers. */
static sqlite3 *g_db = 0;

/* What SQLite answers for one expression. */
static int expr_case(const char *line) {
  char *sql = sqlite3_mprintf("SELECT typeof(%s), quote(%s);", line, line);
  sqlite3_stmt *stmt = 0;
  int rc;
  if (sql == 0) return 1;
  rc = sqlite3_prepare_v2(g_db, sql, -1, &stmt, 0);
  if (rc == SQLITE_OK) rc = sqlite3_step(stmt);
  if (rc == SQLITE_ROW) {
    printf("%s\t%s\n", sqlite3_column_text(stmt, 0),
           sqlite3_column_text(stmt, 1));
  } else {
    const char *message = sqlite3_errmsg(g_db);
    printf("!\t%s\n", message);
  }
  sqlite3_finalize(stmt);
  sqlite3_free(sql);
  return 0;
}

/* The operands every binary operator is put between. */
static const char *aOperand[] = {
  "NULL", "0", "1", "-3", "2.5", "1e308", "9223372036854775807", "''",
  "'abc'", "'2'", "'2.5'", "' 3 '", "'abc9'", "x'32'", "x''",
  "0x7FFFFFFFFFFFFFFF"
};

/* The operators between them. */
static const char *aBinary[] = {
  "+", "-", "*", "/", "%", "||", "&", "|", "<<", ">>", "=", "==", "!=",
  "<>", "<", "<=", ">", ">=", "IS", "IS NOT", "AND", "OR"
};

/* The values the one-operand cases are put to. */
static const char *aSingle[] = {
  "NULL", "0", "1", "-1", "2", "-3", "0.0", "2.5", "-2.5", "1e308",
  "1e-308", "9223372036854775807", "9223372036854775808",
  "-9223372036854775808", "0x10", "0x8000000000000000", "1_000", ".5",
  "1.", "1e3", "''", "'a'", "'A'", "'abc'", "'a''b'", "'1'", "'1.0'",
  "' 1 '", "'10'", "'9'", "'0'", "'abc '", "'0x10'", "' '", "x''",
  "x'41'", "x'4142'", "x'00'", "'1.5'", "'1e400'", "1e400", "-1e400",
  "'9223372036854775808'", "'-9223372036854775809'", "' 12 '", "'-0'",
  "'.5'", "'1e'", "'+3'"
};

/* The type names a cast is written with. */
static const char *aType[] = {
  "INTEGER", "INT", "TEXT", "BLOB", "REAL", "NUMERIC", "VARCHAR(10)",
  "FLOAT", "DOUBLE", "CHARACTER(20)", "", "POINT", "BLOBTEXT",
  "TEXTBLOB", "REALBLOB", "BLOBREAL", "CHARREAL", "REALCHAR", "DECIMAL"
};

/* The cases no cross product writes. */
static const char *aOther[] = {
  "CASE WHEN 1 THEN 'a' ELSE 'b' END",
  "CASE WHEN 0 THEN 'a' ELSE 'b' END",
  "CASE WHEN NULL THEN 'a' ELSE 'b' END",
  "CASE WHEN 0 THEN 'a' END",
  "CASE 1 WHEN 1 THEN 'a' WHEN 2 THEN 'b' ELSE 'c' END",
  "CASE 2 WHEN 1 THEN 'a' WHEN 2 THEN 'b' ELSE 'c' END",
  "CASE 3 WHEN 1 THEN 'a' WHEN 2 THEN 'b' ELSE 'c' END",
  "CASE 3 WHEN 1 THEN 'a' WHEN 2 THEN 'b' END",
  "CASE NULL WHEN NULL THEN 'a' ELSE 'b' END",
  "CASE '1' WHEN 1 THEN 'a' ELSE 'b' END",
  "CASE WHEN 'x' THEN 'a' ELSE 'b' END",
  "2 BETWEEN 1 AND 3", "2 BETWEEN 3 AND 1", "2 NOT BETWEEN 1 AND 3",
  "NULL BETWEEN 1 AND 3", "2 BETWEEN NULL AND 3", "2 BETWEEN 1 AND NULL",
  "0 BETWEEN NULL AND 3", "4 BETWEEN 1 AND NULL",
  "'b' BETWEEN 'a' AND 'c'", "2 BETWEEN '1' AND '3'",
  "1 IN (1,2,3)", "4 IN (1,2,3)", "NULL IN (1,2,3)", "1 IN (NULL,2)",
  "2 IN (NULL,2)", "1 NOT IN (1,2)", "4 NOT IN (1,2)",
  "NULL NOT IN (1,2)", "'1' IN (1,2)", "1 IN ('1','2')",
  "1 IN (1.0,2)", "'a' IN ('A')", "'a' COLLATE NOCASE IN ('A')",
  "'a' = 'A' COLLATE NOCASE", "'a' COLLATE NOCASE = 'A'",
  "'a ' = 'a'", "'a ' = 'a' COLLATE RTRIM", "'a' COLLATE BINARY = 'A'",
  "'B' COLLATE NOCASE < 'a'", "'B' < 'a'",
  "CAST('1' AS INTEGER) = '1'", "CAST(1 AS TEXT) = 1",
  "1 = '1'", "1 = 1.0", "'1' = '1.0'", "1.0 = '1'",
  "9223372036854775807 + 1", "-9223372036854775808 - 1",
  "9223372036854775807 * 2", "-9223372036854775808 / -1",
  "9223372036854775807 % -1", "5 % 0", "5 / 0", "5.0 / 0", "5 % 0.0",
  "5.5 % 2", "-5 % 3", "5 % -3", "-5.5 % 2",
  "1 << 64", "-1 >> 64", "1 >> -1", "1 << -1", "-1 << 1", "-8 >> 2",
  "1 << 63", "1 << 62", "-1 >> 1", "~0", "~-1", "~'abc'", "~2.9",
  "NOT 0", "NOT 1", "NOT NULL", "NOT 'abc'", "NOT ''", "NOT 0.0",
  "NULL ISNULL", "1 ISNULL", "NULL NOTNULL", "1 NOTNULL",
  "1 IS NULL", "1 IS NOT NULL", "NULL IS NULL", "NULL IS NOT NULL",
  "-(-9223372036854775808)", "-(9223372036854775807)", "- -1", "+ -1",
  "+'abc'", "-'abc'", "-'2abc'", "-x'32'", "- (1+1)",
  "0.1 + 0.2", "1.0 / 3", "1 / 3", "1 / 3.0", "2 * 0.5",
  "'5' + 5", "'5abc' + 5", "'abc' + 5", "'' + 5", "' 5 ' + 5",
  "'5' || 5", "5 || 5", "x'41' || 'b'", "NULL || 'a'",
  "1 AND NULL", "0 AND NULL", "1 OR NULL", "0 OR NULL",
  "NULL AND NULL", "NULL OR NULL", "'a' AND 1", "'1' AND 1",
  "1 < 2 < 3", "3 > 2 > 1", "1 = 1 = 1",
  "1 IN ()", "1 NOT IN ()", "NULL IN ()", "NULL NOT IN ()",
  "'9223372036854775808' + 0", "'9223372036854775808' * 1",
  "'-9223372036854775809' + 0", "'9223372036854775807' + 0",
  "1e400 - 1e400", "1e400 * 0", "1e400 / 1e400", "1e400 + 1e400",
  "-1e400 + 1e400", "'1e400' + 0", "1e400 > 1", "1e400 = 1e400",
  "CAST('abc' AS INTEGER) = 'abc'", "CAST(1 AS REAL) = '1.5'",
  "CAST('1.5' AS NUMERIC) = '1.5'", "CAST(1 AS TEXT) < 2",
  "CAST('x' AS TEXT) = 1", "CAST(1 AS BLOB) = '1'",
  "CAST(1.5 AS INTEGER)", "CAST(-1.5 AS INTEGER)", "CAST(1e400 AS INTEGER)",
  "CAST(-1e400 AS INTEGER)", "CAST(1e400 AS NUMERIC)",
  "CAST('12' AS INTEGER) < CAST('9' AS INTEGER)",
  "'ABC' COLLATE NOCASE = 'abc'", "'abc' COLLATE RTRIM = 'abc   '",
  "'abc' COLLATE NOCASE COLLATE BINARY = 'ABC'",
  "'a' < 'b' COLLATE NOCASE", "x'41' = x'41'", "x'41' < x'42'",
  "x'41' < x'4100'", "'' < 'a'", "'a' < 'ab'",
  "abs(-9223372036854775808)", "abs(9223372036854775807)",
  "abs('abc')", "abs(x'32')", "substr('abcdef',-2)",
  "substr('abcdef',0,2)", "substr('abcdef',2,-1)",
  "substr(x'0102030405',2,2)", "substr('abcdef',-100,2)",
  "substr('abcdef',-100,200)", "substr('abcdef',3)",
  "substr('abcdef',-2,-1)", "substr('\u00e4\u00f6\u00fc',2,1)",
  "round(2.5)", "round(-2.5)", "round(2.345,2)", "round(1e400)",
  "round(2.5,-1)", "round(0.0004,2)", "round(123.456,30)",
  "round(1e300,2)", "round(-0.5)", "round(0.5)", "round('abc')",
  "trim('xxayyax','xya')", "trim('  a  ')", "ltrim('  a  ')",
  "rtrim('  a  ')", "trim('\u00e4a\u00e4','\u00e4')",
  "instr('abc','')", "instr(x'0102',x'02')", "instr('abc','c')",
  "instr('\u00e4bc','b')", "unhex('4 1')", "unhex('zz')",
  "unhex('4a')", "unhex('4 1',' ')", "unhex('')", "unhex('4')",
  "char(65,66)", "char()", "char(-1)", "char(1114112)", "char(0)",
  "unicode('\u00e4')", "unicode('')", "concat()", "coalesce(1)",
  "abs(1,2)", "substr('abc')", "like('a','b','c','d')", "nosuchfunc(1)",
  "'a' LIKE 'b' ESCAPE 'xy'", "'a' LIKE 'b' ESCAPE ''",
  "'abc' REGEXP 'a'", "'abc' MATCH 'a'",
  "min('a','B')", "max('a','B')", "min(1,'1')", "max(2,'10')",
  "nullif('a','A')", "quote(x'00')",
  "length(x'000102')", "octet_length(x'000102')",
  "concat_ws('-',1,2)", "concat_ws('-',1,2,3)", "concat_ws('',1,2)",
  "unhex('41 ',' ')", "unhex(' 41',' ')", "unhex('4 1 4 2',' ')",
  "likelihood(1,0.5)", "likelihood('a',1.0)", "likelihood(1,0.0)",
  "likelihood(1,1.5)", "likelihood(1,-0.5)",
  "NULL LIKE 'a'", "'a' LIKE NULL", "NULL LIKE NULL", "NULL GLOB 'a'",
  "'a' GLOB NULL", "NULL NOT LIKE 'a'",
  "'a' LIKE 'b' ESCAPE NULL", "'a%b' LIKE 'a%b' ESCAPE '%'",
  "'a_b' LIKE 'a_b' ESCAPE '_'", "'axb' LIKE 'a_b' ESCAPE '_'",
  "'a' LIKE 'a\\' ESCAPE '\\'", "'A' LIKE '\\a' ESCAPE '\\'",
  "'a' LIKE '\\A' ESCAPE '\\'", "'axb' LIKE 'a\\_b' ESCAPE '\\'",
  "'a_b' LIKE 'a\\_b' ESCAPE '\\'", "'a' GLOB '*\\'",
  "'a' LIKE '%\\' ESCAPE '\\'", "'ab' LIKE '%\\' ESCAPE '\\'",
  "'a' LIKE '_%\\' ESCAPE '\\'",
  "'ab' LIKE 'a%' ESCAPE '_'", "'ab' LIKE '%' ESCAPE '_'",
  "'a_b' LIKE 'a%_b' ESCAPE '_'", "'ab' LIKE 'a%' ESCAPE '%'",
  "'a' GLOB '[-a]'", "'-' GLOB '[-a]'", "'a' GLOB '[a-'",
  "'-' GLOB '[a-'", "'a' GLOB '[a-]'", "'-' GLOB '[a-]'",
  "'b' GLOB '[a-c]'", "'a' GLOB '[]a]'", "']' GLOB '[]a]'",
  "'a' GLOB '[^]a]'", "'a' GLOB '[abc'",
  "'ab' LIKE '%\\b' ESCAPE '\\'", "'a\\b' LIKE '%\\\\b' ESCAPE '\\'",
  "'aXb' LIKE '%\\_b' ESCAPE '\\'",
  "unicode(CAST(x'EDA080' AS TEXT))", "unicode(CAST(x'C081' AS TEXT))",
  "unicode(CAST(x'EFBFBE' AS TEXT))", "unicode(CAST(x'80' AS TEXT))",
  "length(CAST(x'EDA080' AS TEXT))", "char(200)", "char(70000)",
  "char(2048)", "char(55296)", "round(0.06,1)", "round(0.04,1)",
  "round(0.6,0)", "round(-0.06,1)", "round(9.95,1)", "round(0.005,2)",
  "-9223372036854775809", "-99999999999999999999", "-(1e400)",
  "5.5 % -1", "-5.5 % -1", "'5.5' % -1", "5.5 % 1", "2.5 % -2",
  "1 << -64", "1 >> -100", "-1 << -64", "1 << -63",
  "4 BETWEEN 1 AND 3", "2 BETWEEN 1 AND 1", "0 BETWEEN 1 AND 3",
  "'99999999999999999999abc' + 0", "'12abc' * 2", "' 12 ' + 0",
  "'9223372036854775808abc' + 0", "'0x10' + 0", "'1.5abc' + 0"
};

/* The functions of one argument. */
static const char *aUnary[] = {
  "typeof", "length", "octet_length", "abs", "sign", "hex", "unhex",
  "unicode", "quote", "lower", "upper", "trim", "ltrim", "rtrim", "round",
  "likely", "unlikely", "char", "min", "max", "concat"
};

/* The functions of two arguments. */
static const char *aBinaryFunc[] = {
  "instr", "substr", "nullif", "ifnull", "min", "max", "like", "glob",
  "round", "trim", "ltrim", "rtrim", "unhex", "likelihood", "concat",
  "concat_ws", "iif", "coalesce"
};

/* The values put to them, kept short so the cross product stays small. */
static const char *aArg[] = {
  "NULL", "0", "2", "-2", "2.5", "'abc'", "'2'", "''", "x'4142'", "x''",
  "'AB'", "'  a  '"
};

/* Patterns and the strings they are matched against. */
static const char *aPattern[] = {
  "''", "'a'", "'A'", "'%'", "'_'", "'a%'", "'%a'", "'%a%'", "'a_c'",
  "'%%'", "'_%'", "'%_'", "'a%b'", "'[abc]'", "'[a-c]'", "'[^a]'", "'*'",
  "'?'", "'a*c'", "'['", "']'", "'[]]'", "'[a-]'", "'*[bc]*'", "'\\%'",
  "'a\\%b'", "'%\\_%'", "'\u00e4'", "'%\u00e4%'", "'??'", "'a?c'"
};

static const char *aSubject[] = {
  "''", "'a'", "'A'", "'abc'", "'ABC'", "'a%b'", "'a_b'", "'\u00e4'",
  "'aa'", "'ab'", "'abcd'", "'[a]'", "']'", "'a-c'", "'%'"
};

/* The cases, as SQL text, one per line. */
static void expr_corpus(void) {
  int i, j, k;
  int nOperand = (int)(sizeof(aOperand) / sizeof(aOperand[0]));
  int nBinary = (int)(sizeof(aBinary) / sizeof(aBinary[0]));
  int nSingle = (int)(sizeof(aSingle) / sizeof(aSingle[0]));
  int nType = (int)(sizeof(aType) / sizeof(aType[0]));
  for (i = 0; i < nSingle; i++) {
    printf("%s\n", aSingle[i]);
    printf("-(%s)\n", aSingle[i]);
    printf("+(%s)\n", aSingle[i]);
    printf("~(%s)\n", aSingle[i]);
    printf("NOT (%s)\n", aSingle[i]);
    printf("(%s) ISNULL\n", aSingle[i]);
    for (k = 0; k < nType; k++) {
      printf("CAST((%s) AS %s)\n", aSingle[i], aType[k]);
    }
  }
  for (i = 0; i < nOperand; i++) {
    for (j = 0; j < nOperand; j++) {
      for (k = 0; k < nBinary; k++) {
        printf("(%s) %s (%s)\n", aOperand[i], aBinary[k], aOperand[j]);
      }
    }
  }
  for (i = 0; i < (int)(sizeof(aOther) / sizeof(aOther[0])); i++) {
    printf("%s\n", aOther[i]);
  }
  {
    int nArg = (int)(sizeof(aArg) / sizeof(aArg[0]));
    int nUnary = (int)(sizeof(aUnary) / sizeof(aUnary[0]));
    int nBinaryFunc = (int)(sizeof(aBinaryFunc) / sizeof(aBinaryFunc[0]));
    int nPattern = (int)(sizeof(aPattern) / sizeof(aPattern[0]));
    int nSubject = (int)(sizeof(aSubject) / sizeof(aSubject[0]));
    for (i = 0; i < nUnary; i++) {
      for (j = 0; j < nArg; j++) {
        printf("%s(%s)\n", aUnary[i], aArg[j]);
      }
    }
    for (i = 0; i < nBinaryFunc; i++) {
      for (j = 0; j < nArg; j++) {
        for (k = 0; k < nArg; k++) {
          printf("%s(%s,%s)\n", aBinaryFunc[i], aArg[j], aArg[k]);
        }
      }
    }
    for (i = 0; i < nArg; i++) {
      for (j = 0; j < nArg; j++) {
        for (k = 0; k < nArg; k++) {
          printf("substr(%s,%s,%s)\n", aArg[i], aArg[j], aArg[k]);
          printf("replace(%s,%s,%s)\n", aArg[i], aArg[j], aArg[k]);
          printf("iif(%s,%s,%s)\n", aArg[i], aArg[j], aArg[k]);
        }
      }
    }
    for (i = 0; i < nPattern; i++) {
      for (j = 0; j < nSubject; j++) {
        printf("%s LIKE %s\n", aSubject[j], aPattern[i]);
        printf("%s GLOB %s\n", aSubject[j], aPattern[i]);
        printf("%s NOT LIKE %s\n", aSubject[j], aPattern[i]);
        printf("%s LIKE %s ESCAPE '\\'\n", aSubject[j], aPattern[i]);
      }
    }
  }
}

int main(int argc, char **argv) {
  char line[4096];
  if (argc != 2) {
    fprintf(stderr, "usage: sqlite-oracle <mode>; see the head of this file\n");
    return 2;
  }
  if (strcmp(argv[1], "fp-corpus") == 0) {
    fp_corpus();
    return 0;
  }
  if (strcmp(argv[1], "num-corpus") == 0) {
    num_corpus();
    return 0;
  }
  if (strcmp(argv[1], "expr-corpus") == 0) {
    expr_corpus();
    return 0;
  }
  if (strcmp(argv[1], "expr") == 0) {
    int rc = sqlite3_open(":memory:", &g_db);
    if (rc != SQLITE_OK) {
      fprintf(stderr, "sqlite-oracle: cannot open a database\n");
      return 1;
    }
    while (fgets(line, sizeof(line), stdin) != 0) {
      size_t len = strlen(line);
      while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r')) {
        line[--len] = 0;
      }
      if (len == 0) continue;
      if (expr_case(line) != 0) return 1;
    }
    sqlite3_close(g_db);
    return 0;
  }
  if (strcmp(argv[1], "fp") != 0 && strcmp(argv[1], "num") != 0) {
    fprintf(stderr, "sqlite-oracle: unknown mode %s\n", argv[1]);
    return 2;
  }
  while (fgets(line, sizeof(line), stdin) != 0) {
    size_t len = strlen(line);
    int bad;
    while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r')) {
      line[--len] = 0;
    }
    if (len == 0) continue;
    bad = argv[1][0] == 'f' ? fp_case(line) : num_case(line);
    if (bad != 0) {
      fprintf(stderr, "sqlite-oracle: bad case %s\n", line);
      return 1;
    }
  }
  return 0;
}
