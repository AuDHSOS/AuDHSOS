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
**   schema-corpus  writes the cases for the reader of schemas.
**   legacy      a path and the statements to run there, written under
**               the schema format the file format document calls 1,
**               which `SQLITE_DBCONFIG_LEGACY_FILE_FORMAT` asks for and
**               no pragma the shell takes does. Answers the format the
**               file ends with, which ALTER TABLE ADD COLUMN raises.
**   query-corpus   writes the cases for the engine that answers a query.
**   query       a fixture and a statement, separated by a bar, answered
**               as the rows the statement makes of that file. The
**               second argument names the directory the fixtures are in.
**   schema      one CREATE statement, answered as what the schema holds
**               after it: the table with its columns, or the index with
**               its terms. A refusal is answered as "!" and whether it
**               is a syntax error, which is what a parser alone decides.
*/
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "sqlite3.h"

/* Two routines the amalgamation keeps to itself. The build defines
** SQLITE_PRIVATE to nothing so that they can be asked directly. */
extern int sqlite3AtoF(const char *, double *);
extern int sqlite3Atoi64(const char *, sqlite3_int64 *, int, unsigned char);
extern void sqlite3QuoteValue(sqlite3_str *, sqlite3_value *, int);

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
  /* The math functions whose answers are exact: a whole number, a
  ** constant, or one multiply. */
  "pi()", "degrees(1.0)", "degrees(0.5)", "degrees(-1.0)", "degrees(0)",
  "degrees(1e308)", "degrees('2')", "degrees('x')", "degrees(NULL)",
  "degrees(x'01')", "radians(180.0)", "radians(1)", "radians(-90)",
  "radians(1e-308)", "radians('180')",
  "ceil(1.2)", "ceil(-1.2)", "ceil(1.0)", "ceil(-0.5)", "ceil(0.5)",
  "ceil(0.0)", "ceil(-0.0)", "ceil(7)", "ceil(-7)", "ceil('2.5')",
  "ceil('x')", "ceil(NULL)", "ceil(x'01')", "ceil(9007199254740993.0)",
  /* `unistr` reads the escapes `%#q` writes, one form per line. */
  "unistr('\\0041')", "unistr('\\u0041')", "unistr('\\U00000041')",
  "unistr('\\+000041')", "unistr('\\\\')", "unistr('a\\\\b')",
  "unistr('\\00e4')", "unistr('\\d83d\\de00')", "unistr('\\U0001d11e')",
  "unistr('\\U7fffffff')", "unistr('\\0000')", "unistr('a\\0041b')",
  "unistr('\\0041\\0042')", "unistr('x')", "unistr('')",
  "unistr('\\xyz')", "unistr('\\')", "unistr('\\004')", "unistr('\\u12')",
  "unistr('\\+00')", "unistr('\\U1234')", "unistr('\\uABCD')",
  "unistr('\\uabcd')", "unistr('a\\')", "unistr(5)", "unistr(2.5)",
  "unistr(NULL)", "unistr(x'5C753030343100')",
  "unistr_quote('a''b')", "unistr_quote(char(1)||'x')",
  "unistr_quote('a\\\\b')", "unistr_quote(2.5)", "unistr_quote(NULL)",
  "unistr_quote(x'41')", "unistr_quote(9e999)", "unistr_quote(-9e999)",
  "unistr_quote('')", "quote(char(1)||'x')", "quote(9e999)",
  "unistr(unistr_quote('a'))",
  "ceil(1e308)", "ceil(-1e308)", "ceil(4503599627370495.5)",
  "ceiling(1.2)", "ceiling(-1.2)",
  "floor(1.2)", "floor(-1.2)", "floor(1.0)", "floor(-0.5)", "floor(0.5)",
  "floor(0.0)", "floor(-0.0)", "floor(7)", "floor(-7)", "floor('2.5')",
  "floor('x')", "floor(NULL)", "floor(4503599627370495.5)",
  "trunc(1.8)", "trunc(-1.8)", "trunc(0.9)", "trunc(-0.9)", "trunc(7)",
  "trunc(-7)", "trunc('2.9')", "trunc(NULL)", "trunc(1e308)",
  "trunc(4503599627370495.5)", "trunc(-4503599627370495.5)",
  "quote(zeroblob(0))", "quote(zeroblob(3))", "quote(zeroblob(-1))",
  "length(zeroblob(4))", "typeof(zeroblob(1))", "zeroblob(NULL)",
  "quote(zeroblob('2'))", "quote(zeroblob(2.9))", "hex(zeroblob(2))",
  "mod(7,3)", "mod(7.5,2.0)", "mod(-7.5,2.0)", "mod(7.5,-2.0)",
  "mod(-7.5,-2.0)", "mod(7,0)", "mod(7.0,0.0)", "mod(0,3)", "mod(0.0,3.0)",
  "mod(-0.0,3.0)", "mod(1,3)", "mod(1e308,3.0)", "mod(1e308,1e-308)",
  "mod(3.0,7.0)", "mod(1e-308,1e308)", "mod(5e-324,1e300)",
  "mod(1e300,5e-324)", "mod(9223372036854775807,2)", "mod('7','3')",
  "mod('x',3)", "mod(3,'x')", "mod(NULL,3)", "mod(3,NULL)",
  "mod(x'01',3)", "mod(4.5,1.5)", "mod(-1.0,0.5)",
  "mod(1e400,3.0)", "mod(3.0,1e400)", "mod(1e400,1e400)",
  "mod(-1e400,3.0)", "mod(1e400,0.0)",
  "changes()", "total_changes()", "last_insert_rowid()",
  "typeof(changes())", "changes()+1",
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
  "unhex(' ' || char(0) || '41',' ')", "unhex(char(0) || '41')",
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
  "unicode", "quote", "unistr", "unistr_quote", "lower", "upper", "trim",
  "ltrim", "rtrim", "round", "likely", "unlikely", "char", "min", "max",
  "concat"
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

/* The formats `format(F,...)` is put through. */
static const char *aFormat[] = {
  "'%d'", "'%i'", "'%u'", "'%5d'", "'%-5d'", "'%05d'", "'%+d'", "'% d'",
  "'%.3d'", "'%.0d'", "'%,d'", "'%,.10d'", "'%8,d'", "'%09,d'",
  "'%x'", "'%X'", "'%#x'", "'%#X'", "'%08x'", "'%o'", "'%#o'", "'%p'",
  "'%r'", "'%-8r'", "'%.5r'", "'%+r'",
  "'%f'", "'%.0f'", "'%.10f'", "'%!f'", "'%#.0f'", "'%,f'", "'%012.2f'",
  "'%-12.2f'", "'%+.2f'", "'%0f'", "'%#f'", "'%!.3f'", "'%,.3f'",
  "'%e'", "'%E'", "'%.0e'", "'%!e'", "'%+15.4e'", "'%-15.4E'", "'%,e'",
  "'%g'", "'%G'", "'%.1g'", "'%.17g'", "'%!.20g'", "'%#g'", "'%20.3g'",
  "'%.0g'", "'%,g'", "'%030.8g'", "'%!g'",
  "'%s'", "'%10s'", "'%-10s'", "'%.2s'", "'%!.2s'", "'%!6s'", "'%z'",
  "'%.0s'", "'%!-6s'", "'%08s'",
  "'%q'", "'%Q'", "'%w'", "'%#q'", "'%#Q'", "'%#w'", "'%.2q'", "'%8q'",
  "'%!.2Q'", "'%-9w'", "'%.0Q'",
  "'%c'", "'%3c'", "'%.3c'", "'%-6.3c'", "'%6.3c'", "'%.1c'", "'%2.4c'",
  "'%%'", "'%5%'", "'%n'", "'%y'", "'a%Tb'", "'a%Sb'", "'%'", "'%l'",
  "'%ld'", "'%lld'", "'%.3lf'", "'%5.2lld'", "'%.l'", "'%5l'",
  "'[%s][%d]'", "'%5.2f%%'", "'%s%s'", "'%d%d%d'", "'no conversion'", "''",
  "'%,x'", "'%-'", "'%+'", "'%!c'", "'%!q'", "'%!.4c'", "'%+#f'", "'% #g'",
  "'%+#e'"
};

/* The values the formats are put to. */
static const char *aFormatArg[] = {
  "NULL", "0", "1", "-1", "10", "255", "1000000", "-9223372036854775808",
  "9223372036854775807", "2.5", "-0.0625", "1e300", "1e-300", "9e999",
  "-9e999", "'abc'", "''", "'a''b'", "'\\'", "x'414243'", "'käse'",
  "char(1)||'x'", "'123abc'", "0.0", "char(17)||'x'", "'äbc'",
  "'𝄞x'"
};

/* The values every pair of arguments is drawn from. */
static const char *aFormatPair[] = {"1", "-5", "2.5", "'abc'", "NULL"};

/* The formats whose width or precision is an argument of its own. */
static const char *aFormatStar[] = {
  "'%*d'", "'%-*d'", "'%.*f'", "'%*.*f'", "'%*s'", "'%.*s'", "'%*.*g'",
  "'%*c'", "'%.*q'", "'%0*d'", "'%*ld'", "'%.*lf'"
};

/* The widths and precisions handed to them. */
static const char *aFormatWidth[] = {
  "0", "1", "8", "-8", "2147483648", "-2147483648", "NULL"
};

/* The values handed to them. */
static const char *aFormatValue[] = {"1", "-2.5", "'abcdef'"};

/* The cases whose width or precision is more than any answer holds.
** Each is refused, and neither engine allocates what it asks for; a
** width the C library does fill, such as `%999999900.5f` of a small
** number, would write a gigabyte and is left out. */
static const char *aFormatHuge[] = {
  "format('%2000000000d',1)", "format('%-2000000000d',1)",
  "format('%2000000000s','abc')", "format('%2000000000c','a')",
  "format('%2000000000q','a''b')", "format('%900000000.100000000f',1.5)",
  "format('%2000000000.2e',1.5)", "format('%900000000.100000000g',1.5)",
  "format('%02000000000d',-1)", "format('%2000000000%')",
  "format('%999999900.5f',1e300)", "format('%0999999900f',9e999)",
  "format('%900000000.1000000000e',1.5)", "format('%2000000000x',255)",
  "format('%.2000000000c','a')", "format('%.2000000000q','ab')",
  "format('%2000000000.5c','a')"
};

/* The cases for the format language, as SQL text, one per line. */
static void format_corpus(void) {
  int i, j, k;
  int nFormat = (int)(sizeof(aFormat) / sizeof(aFormat[0]));
  int nArg = (int)(sizeof(aFormatArg) / sizeof(aFormatArg[0]));
  int nPair = (int)(sizeof(aFormatPair) / sizeof(aFormatPair[0]));
  int nStar = (int)(sizeof(aFormatStar) / sizeof(aFormatStar[0]));
  int nWidth = (int)(sizeof(aFormatWidth) / sizeof(aFormatWidth[0]));
  int nValue = (int)(sizeof(aFormatValue) / sizeof(aFormatValue[0]));
  for (i = 0; i < nFormat; i++) {
    printf("format(%s)\n", aFormat[i]);
    for (j = 0; j < nArg; j++) {
      printf("format(%s,%s)\n", aFormat[i], aFormatArg[j]);
    }
    for (j = 0; j < nPair; j++) {
      for (k = 0; k < nPair; k++) {
        printf("format(%s,%s,%s)\n", aFormat[i], aFormatPair[j],
               aFormatPair[k]);
      }
    }
  }
  for (i = 0; i < nStar; i++) {
    for (j = 0; j < nWidth; j++) {
      for (k = 0; k < nValue; k++) {
        printf("format(%s,%s,%s)\n", aFormatStar[i], aFormatWidth[j],
               aFormatValue[k]);
        printf("format(%s,%s,3,%s)\n", aFormatStar[i], aFormatWidth[j],
               aFormatValue[k]);
      }
    }
  }
  /* The same function under its other name, and the format itself as a
  ** value of every class. */
  for (i = 0; i < nArg; i++) {
    printf("printf('%%d %%s',%s,%s)\n", aFormatArg[i], aFormatArg[i]);
    printf("format(%s,1,2)\n", aFormatArg[i]);
  }
  for (i = 0; i < (int)(sizeof(aFormatHuge) / sizeof(aFormatHuge[0])); i++) {
    printf("%s\n", aFormatHuge[i]);
  }
  printf("format()\n");
}

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
  format_corpus();
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

/* The table every index case is written against. */
static const char *zBase =
  "CREATE TABLE base(a, b INTEGER, c TEXT COLLATE NOCASE, d REAL);";

/* One query, its rows printed as tab-separated fields after `zTag`. */
static int schema_dump(sqlite3 *db, const char *zTag, const char *zSql) {
  sqlite3_stmt *stmt = 0;
  int rc = sqlite3_prepare_v2(db, zSql, -1, &stmt, 0);
  if (rc != SQLITE_OK) {
    sqlite3_finalize(stmt);
    return 1;
  }
  while (sqlite3_step(stmt) == SQLITE_ROW) {
    int i, n = sqlite3_column_count(stmt);
    printf("%s", zTag);
    for (i = 0; i < n; i++) {
      const unsigned char *text = sqlite3_column_text(stmt, i);
      printf("|%s", text ? (const char *)text : "");
    }
  }
  sqlite3_finalize(stmt);
  return 0;
}

/* The name of the object the statement made, and its type, into zType
** and zName. Answers 0 where it made none. */
static int schema_made(sqlite3 *db, sqlite3_int64 before, char *zType,
                       char *zName) {
  static const char *azFrom[] = {"sqlite_schema", "sqlite_temp_schema"};
  int at;
  for (at = 0; at < 2; at++) {
    sqlite3_stmt *stmt = 0;
    char *sql = sqlite3_mprintf(
        "SELECT type, name FROM %s WHERE rowid>%lld AND sql IS NOT NULL"
        " ORDER BY rowid LIMIT 1", azFrom[at], (long long)(at ? 0 : before));
    int found = 0;
    if (sql == 0) return 0;
    if (sqlite3_prepare_v2(db, sql, -1, &stmt, 0) == SQLITE_OK
     && sqlite3_step(stmt) == SQLITE_ROW) {
      sqlite3_snprintf(64, zType, "%s", sqlite3_column_text(stmt, 0));
      sqlite3_snprintf(256, zName, "%s", sqlite3_column_text(stmt, 1));
      found = 1;
    }
    sqlite3_finalize(stmt);
    sqlite3_free(sql);
    if (found) return 1;
  }
  return 0;
}

/* What the schema holds after one CREATE statement. */
static int schema_case(const char *line) {
  sqlite3 *db = 0;
  char *message = 0;
  char zType[64], zName[256], *sql;
  sqlite3_int64 before = 0;
  sqlite3_stmt *stmt = 0;
  int rc;
  if (sqlite3_open(":memory:", &db) != SQLITE_OK) return 1;
  sqlite3_exec(db, zBase, 0, 0, 0);
  if (sqlite3_prepare_v2(db, "SELECT max(rowid) FROM sqlite_schema", -1, &stmt,
                         0) == SQLITE_OK
   && sqlite3_step(stmt) == SQLITE_ROW) {
    before = sqlite3_column_int64(stmt, 0);
  }
  sqlite3_finalize(stmt);
  rc = sqlite3_exec(db, line, 0, 0, &message);
  if (rc != SQLITE_OK) {
    /* A refusal the grammar itself made says `near "X": syntax error`.
    ** Everything else is a rule the grammar's actions apply, which a
    ** parser alone does not decide. */
    const char *kind = "other";
    if (message && strncmp(message, "near \"", 6) == 0
     && strstr(message, "syntax error") != 0) {
      kind = "syntax";
    }
    printf("!\t%s\t%s\n", kind, message ? message : "");
    sqlite3_free(message);
    sqlite3_close(db);
    return 0;
  }
  if (!schema_made(db, before, zType, zName)) {
    printf("none\n");
    sqlite3_close(db);
    return 0;
  }
  printf("%s", zType);
  if (strcmp(zType, "table") == 0) {
    sql = sqlite3_mprintf(
        "SELECT name, ncol, wr, strict FROM pragma_table_list WHERE name=%Q",
        zName);
    schema_dump(db, "\tT", sql);
    sqlite3_free(sql);
    sql = sqlite3_mprintf(
        "SELECT cid, name, type, \"notnull\", ifnull(dflt_value,'~'), pk, hidden"
        " FROM pragma_table_xinfo(%Q)", zName);
    schema_dump(db, "\tC", sql);
    sqlite3_free(sql);
  } else if (strcmp(zType, "index") == 0) {
    sql = sqlite3_mprintf(
        "SELECT il.name, il.\"unique\", il.origin, il.partial"
        " FROM sqlite_schema s JOIN pragma_index_list(s.tbl_name) il"
        " ON il.name=s.name WHERE s.name=%Q", zName);
    schema_dump(db, "\tI", sql);
    sqlite3_free(sql);
    sql = sqlite3_mprintf(
        "SELECT seqno, ifnull(name,'~'), \"desc\", coll, key"
        " FROM pragma_index_xinfo(%Q)", zName);
    schema_dump(db, "\tX", sql);
    sqlite3_free(sql);
  }
  printf("\n");
  sqlite3_close(db);
  return 0;
}

/* The cases: the shapes a schema is written in, and the ways of writing
** each of them wrong. */
static const char *aSchema[] = {
  "CREATE TABLE t(x)",
  "CREATE TABLE t(x INTEGER)",
  "CREATE TABLE t(x INT, y TEXT, z BLOB, w REAL, v NUMERIC)",
  "CREATE TABLE t(x VARCHAR(10))",
  "CREATE TABLE t(x DECIMAL(10,5))",
  "CREATE TABLE t(x UNSIGNED BIG INT)",
  "CREATE TABLE t(x DOUBLE PRECISION)",
  "CREATE TABLE t(x NATIVE CHARACTER(70))",
  "CREATE TABLE t(x \"quoted type\")",
  "CREATE TABLE t(x 'string type')",
  "CREATE TABLE t(x generated)",
  "CREATE TABLE t(x key)",
  "CREATE TABLE t(x without)",
  "CREATE TABLE t(generated)",
  "CREATE TABLE t(\"select\")",
  "CREATE TABLE t(x, y AS (x+1))",
  "CREATE TABLE t(x, y AS (x+1) STORED)",
  "CREATE TABLE t(x, y AS (x+1) VIRTUAL)",
  "CREATE TABLE t(x, y GENERATED ALWAYS AS (x+1))",
  "CREATE TABLE t(x, y GENERATED ALWAYS AS (x+1) STORED)",
  "CREATE TABLE t(x, y AS (x+1) NONSENSE)",
  "CREATE TABLE t(x GENERATED ALWAYS AS (1))",
  "CREATE TABLE t(x INTEGER PRIMARY KEY)",
  "CREATE TABLE t(x INTEGER PRIMARY KEY ASC)",
  "CREATE TABLE t(x INTEGER PRIMARY KEY DESC)",
  "CREATE TABLE t(x INTEGER PRIMARY KEY AUTOINCREMENT)",
  "CREATE TABLE t(x PRIMARY KEY AUTOINCREMENT)",
  "CREATE TABLE t(x INTEGER PRIMARY KEY ON CONFLICT ROLLBACK)",
  "CREATE TABLE t(x INTEGER PRIMARY KEY ON CONFLICT ABORT AUTOINCREMENT)",
  "CREATE TABLE t(x PRIMARY KEY)",
  "CREATE TABLE t(x TEXT PRIMARY KEY)",
  "CREATE TABLE t(x NOT NULL)",
  "CREATE TABLE t(x NOT NULL ON CONFLICT FAIL)",
  "CREATE TABLE t(x NULL)",
  "CREATE TABLE t(x UNIQUE)",
  "CREATE TABLE t(x UNIQUE ON CONFLICT IGNORE)",
  "CREATE TABLE t(x CHECK(x>0))",
  "CREATE TABLE t(x DEFAULT 1)",
  "CREATE TABLE t(x DEFAULT -1)",
  "CREATE TABLE t(x DEFAULT +1)",
  "CREATE TABLE t(x DEFAULT 1.5)",
  "CREATE TABLE t(x DEFAULT 'a')",
  "CREATE TABLE t(x DEFAULT abc)",
  "CREATE TABLE t(x DEFAULT NULL)",
  "CREATE TABLE t(x DEFAULT x'41')",
  "CREATE TABLE t(x DEFAULT (1+1))",
  "CREATE TABLE t(x DEFAULT CURRENT_TIMESTAMP)",
  "CREATE TABLE t(x DEFAULT 1+1)",
  "CREATE TABLE t(x COLLATE NOCASE)",
  "CREATE TABLE t(x COLLATE BINARY)",
  "CREATE TABLE t(x COLLATE nosuch)",
  "CREATE TABLE t(x REFERENCES base)",
  "CREATE TABLE t(x REFERENCES base(a))",
  "CREATE TABLE t(x REFERENCES base(a) ON DELETE CASCADE)",
  "CREATE TABLE t(x REFERENCES base(a) ON UPDATE SET NULL ON DELETE RESTRICT)",
  "CREATE TABLE t(x REFERENCES base(a) MATCH FULL ON DELETE NO ACTION)",
  "CREATE TABLE t(x REFERENCES base(a) DEFERRABLE INITIALLY DEFERRED)",
  "CREATE TABLE t(x REFERENCES base(a) NOT DEFERRABLE INITIALLY IMMEDIATE)",
  "CREATE TABLE t(x REFERENCES base ON DELETE SET DEFAULT)",
  "CREATE TABLE t(x CONSTRAINT c NOT NULL)",
  "CREATE TABLE t(x, CONSTRAINT c CHECK(x>0))",
  "CREATE TABLE t(x, PRIMARY KEY(x))",
  "CREATE TABLE t(x, y, PRIMARY KEY(x, y))",
  "CREATE TABLE t(x, y, PRIMARY KEY(x DESC, y ASC))",
  "CREATE TABLE t(x, PRIMARY KEY(x) ON CONFLICT REPLACE)",
  "CREATE TABLE t(x INTEGER, PRIMARY KEY(x AUTOINCREMENT))",
  "CREATE TABLE t(x, y, UNIQUE(x, y))",
  "CREATE TABLE t(x, y, UNIQUE(x) ON CONFLICT IGNORE)",
  "CREATE TABLE t(x, y, FOREIGN KEY(x) REFERENCES base(a))",
  "CREATE TABLE t(x, y, FOREIGN KEY(x, y) REFERENCES base(a, b) ON DELETE CASCADE)",
  "CREATE TABLE t(x, CHECK(x>0), CHECK(x<10))",
  "CREATE TABLE t(x PRIMARY KEY) WITHOUT ROWID",
  "CREATE TABLE t(x INTEGER PRIMARY KEY) WITHOUT ROWID",
  "CREATE TABLE t(x INT PRIMARY KEY) STRICT",
  "CREATE TABLE t(x INT PRIMARY KEY) STRICT, WITHOUT ROWID",
  "CREATE TABLE t(x INT PRIMARY KEY) WITHOUT ROWID, STRICT",
  "CREATE TABLE t(x) WITHOUT ROWID",
  "CREATE TABLE t(x) STRICT",
  "CREATE TABLE t(x) NONSENSE",
  "CREATE TABLE t(x) WITHOUT NONSENSE",
  "CREATE TEMP TABLE t(x)",
  "CREATE TEMPORARY TABLE t(x)",
  "CREATE TABLE IF NOT EXISTS t(x)",
  "CREATE TABLE main.t(x)",
  "CREATE TABLE t AS SELECT 1 AS x, 'a' AS y",
  "CREATE TABLE t AS SELECT a, b FROM base",
  "CREATE TABLE t()",
  "CREATE TABLE t",
  "CREATE TABLE (x)",
  "CREATE TABLE t(x,)",
  "CREATE TABLE t(x y z)",
  "CREATE TABLE t(x PRIMARY)",
  "CREATE TABLE t(x NOT)",
  "CREATE TABLE t(x DEFAULT)",
  "CREATE TABLE t(x COLLATE)",
  "CREATE TABLE t(x CHECK)",
  "CREATE TABLE t(x CHECK(x)",
  "CREATE TABLE t(x REFERENCES)",
  "CREATE TABLE t(x ON CONFLICT ROLLBACK)",
  "CREATE TABLE t(x UNIQUE ON CONFLICT NONSENSE)",
  "CREATE TABLE t(x REFERENCES base ON DELETE NONSENSE)",
  "CREATE TABLE t(x REFERENCES base ON NONSENSE CASCADE)",
  "CREATE TABLE t(x, PRIMARY KEY)",
  "CREATE TABLE t(x, PRIMARY KEY())",
  "CREATE TABLE t(x, FOREIGN KEY(x))",
  "CREATE TABLE t(x, FOREIGN(x) REFERENCES base)",
  "CREATE TABLE t(x, x)",
  "CREATE TABLE t(x, y AS (1), z AS (2))",
  "CREATE TABLE t(x, PRIMARY KEY(x+1))",
  "CREATE TABLE t(x, PRIMARY KEY('x'))",
  "CREATE TABLE t(x, PRIMARY KEY(x COLLATE NOCASE))",
  "CREATE TABLE t(x, PRIMARY KEY('x' COLLATE NOCASE))",
  "CREATE TABLE t(a, b, PRIMARY KEY(a)) WITHOUT ROWID",
  "CREATE TABLE t(x aaaaaaaaaa always)",
  "CREATE TABLE t(x ab         always)",
  "CREATE TABLE t(x my generated always)",
  "CREATE TABLE t(x GENERATED ALWAYS)",
  "CREATE INDEX i ON base(a)",
  "CREATE INDEX i ON base(a, b)",
  "CREATE INDEX i ON base(a DESC, b ASC)",
  "CREATE INDEX i ON base(a COLLATE NOCASE)",
  "CREATE INDEX i ON base(a+b)",
  "CREATE INDEX i ON base(a) WHERE b > 0",
  "CREATE UNIQUE INDEX i ON base(a)",
  "CREATE UNIQUE INDEX IF NOT EXISTS i ON base(a)",
  "CREATE INDEX main.i ON base(a)",
  "CREATE INDEX i ON base(a NULLS FIRST)",
  "CREATE INDEX i ON nosuch(a)",
  "CREATE INDEX i ON base(nosuch)",
  "CREATE INDEX i ON base()",
  "CREATE INDEX i ON base",
  "CREATE INDEX i base(a)",
  "CREATE INDEX ON base(a)",
  "CREATE UNIQUE TABLE t(x)",
  "CREATE TEMP INDEX i ON base(a)",
  "CREATE VIEW v AS SELECT 1",
  "CREATE TRIGGER tr AFTER INSERT ON base BEGIN SELECT 1; END",
  "DROP TABLE base",
  "SELECT 1",
  "CREATE TABLE t(x, CHECK(x>0),)",
  "CREATE TABLE t(x, CHECK(x>0) CHECK(x<9))",
  "CREATE TABLE t(x, CONSTRAINT c UNIQUE(x) CONSTRAINT d CHECK(x>0))",
  "CREATE TABLE t(x",
  "CREATE TABLE t(x DEFERRABLE)",
  "CREATE TABLE t(x NOT DEFERRABLE)",
  "CREATE TABLE t(x DEFERRABLE INITIALLY DEFERRED)",
  "CREATE TABLE t(x NOT DEFERRABLE INITIALLY IMMEDIATE)",
  "CREATE TABLE t(x DEFERRABLE INITIALLY NONSENSE)",
  "CREATE TABLE t(x DEFAULT",
  "CREATE TABLE t(x DEFAULT CURRENT_TIME)",
  "CREATE TABLE t(x DEFAULT CURRENT_DATE)",
  "CREATE TABLE t(x DEFAULT key)",
  "CREATE TABLE t(x DEFAULT SELECT)",
  "CREATE TABLE t(x DEFAULT ())",
  "CREATE TABLE t(x DEFAULT \"quoted\")",
  "CREATE TABLE t(x UNIQUE ON DELETE CASCADE)",
  "CREATE TABLE t(x UNIQUE ON CONFLICT",
  "CREATE TABLE t(x REFERENCES base DEFERRABLE)",
  "CREATE TABLE t(x REFERENCES base ON INSERT CASCADE)",
  "CREATE TABLE t(x REFERENCES base NOT NULL)",
  "CREATE TABLE t(x REFERENCES base ON UPDATE RESTRICT)",
  "CREATE TABLE t(x REFERENCES base ON UPDATE SET",
  "CREATE TABLE t(x, FOREIGN KEY(x COLLATE NOCASE) REFERENCES base(a))",
  "CREATE TABLE t(x, FOREIGN KEY(x DESC) REFERENCES base(a ASC))",
  "CREATE TABLE full(x)",
  "CREATE TABLE t(left, indexed)",
  "CREATE INDEX i ON base(a) WHERE",
  "CREATE TABLE t(x INT PRIMARY KEY) STRICT, STRICT",
  "CREATE TABLE t(x) WITHOUT ROWID, WITHOUT ROWID"
};

/* The cases, one per line. */
static void schema_corpus(void) {
  int i;
  for (i = 0; i < (int)(sizeof(aSchema) / sizeof(aSchema[0])); i++) {
    printf("%s\n", aSchema[i]);
  }
}

/* The statements put to the fixture databases. */
static const char *aQuery[] = {
  /* The schema format dimension of document 16, section 16.11: format 1
  ** stores the whole numbers 0 and 1 as a byte each where format 4
  ** stores them as serial types 8 and 9, format 1 ignores the DESC of
  ** an index where format 4 keeps it, and a column added after rows
  ** were written answers what it falls back to. */
  "format1.db|SELECT a, b, c FROM t",
  "format1.db|SELECT typeof(a), typeof(b) FROM t",
  "format1.db|SELECT c FROM t ORDER BY c DESC",
  "format1.db|SELECT * FROM t WHERE c='x'",
  "format3.db|SELECT a, b, c, d FROM t",
  "format3.db|SELECT d FROM t WHERE a=0",
  "format3.db|SELECT count(*), sum(d) FROM t",
  "format3.db|SELECT d, e, typeof(e) FROM t",
  "format4.db|SELECT a, b, c, d FROM t",
  "format4.db|SELECT typeof(a), typeof(b) FROM t",
  "format4.db|SELECT d FROM t WHERE a=0",
  "format4.db|SELECT d, e, typeof(e) FROM t",
  /* A `COLLATE` under an operator, a call or a branch of a `CASE` is
  ** the collation the comparison uses, and a sign written before an
  ** expression takes the affinity off it. */
  "small.db|SELECT 'abc'==('ABC'||'')",
  "small.db|SELECT 'abc'==('ABC'||'' COLLATE nocase)",
  "small.db|SELECT 'abc'==(('ABC' COLLATE nocase)||'')",
  "small.db|SELECT 'abc'==('ABC'||upper('' COLLATE nocase))",
  "small.db|SELECT 'abc'==('ABC'||max('' COLLATE nocase,'' COLLATE binary))",
  "small.db|SELECT 'abc'==('ABC'||CASE WHEN 1=2 THEN '' COLLATE binary ELSE '' COLLATE nocase END)",
  "small.db|SELECT 'abc'==('ABC'||CASE WHEN 1=1 THEN '' COLLATE binary ELSE '' COLLATE nocase END)",
  "small.db|SELECT 'abc'==('ABC'||CASE WHEN 1=1 THEN '' ELSE '' END)",
  "small.db|SELECT 'abc'==('ABC'|| -('' COLLATE nocase))",
  "affinity.db|SELECT rowid, xt==+xi, xt==xi, xt==xb FROM t ORDER BY rowid",
  "affinity.db|SELECT rowid, xi==xt, xi==xb, xi==+xt FROM t ORDER BY rowid",
  "affinity.db|SELECT rowid, xr==xt, xr==xb, xr==+xt FROM t ORDER BY rowid",
  "affinity.db|SELECT rowid, xn==xt, xn==xb, xn==+xt FROM t ORDER BY rowid",
  "affinity.db|SELECT typeof(+xi), typeof(+xt) FROM t ORDER BY rowid",
  "affinity.db|SELECT '1', substr(xt,2) AS xt FROM t ORDER BY xt",
  "affinity.db|SELECT '2', substr(xt,2) AS xt FROM t ORDER BY xt COLLATE binary",
  "affinity.db|SELECT '3', substr(xt,2) AS xt FROM t ORDER BY lower(xt)",
  "defaults.db|SELECT a, b, c FROM t",
  "defaults.db|SELECT typeof(b), typeof(c) FROM t",
  "small.db|SELECT * FROM t",
  "small.db|SELECT a FROM t",
  "small.db|SELECT a, b FROM t WHERE a>1",
  "small.db|SELECT rowid, a FROM t",
  "small.db|SELECT oid, _rowid_ FROM t",
  "small.db|SELECT * FROM t ORDER BY a DESC",
  "small.db|SELECT * FROM t ORDER BY a",
  "small.db|SELECT b FROM t ORDER BY 1",
  "small.db|SELECT a AS x FROM t ORDER BY x DESC",
  "small.db|SELECT a+1, b||'x' FROM t",
  "small.db|SELECT DISTINCT typeof(d) FROM t",
  "small.db|SELECT * FROM t LIMIT 2",
  "small.db|SELECT * FROM t LIMIT 1 OFFSET 1",
  "small.db|SELECT * FROM t LIMIT -1",
  "small.db|SELECT * FROM t LIMIT 1, 2",
  "small.db|SELECT * FROM t WHERE b LIKE 't%'",
  "small.db|SELECT * FROM t WHERE b GLOB 't*'",
  "small.db|SELECT typeof(a), typeof(b), typeof(c), typeof(d) FROM t",
  "small.db|SELECT * FROM t AS x WHERE x.a=1",
  "small.db|SELECT t.a FROM t",
  "small.db|SELECT a FROM t WHERE a IN (1,2)",
  "small.db|SELECT a FROM t WHERE b IS NULL",
  "small.db|SELECT a FROM t WHERE d IS NOT NULL",
  "small.db|SELECT count(*) FROM t",
  "small.db|SELECT a FROM t GROUP BY a",
  "small.db|SELECT a FROM t GROUP BY a HAVING a>1",
  "small.db|WITH x AS (SELECT 1) SELECT * FROM t",
  "small.db|SELECT * FROM nosuch",
  "small.db|SELECT a FROM t ORDER BY 99",
  "small.db|SELECT a, b FROM t ORDER BY 2",
  "keys.db|SELECT * FROM r",
  "keys.db|SELECT id, v FROM r ORDER BY id",
  "keys.db|SELECT rowid, id FROM r",
  "keys.db|SELECT * FROM r WHERE id=9",
  "keys.db|SELECT * FROM w",
  "keys.db|SELECT a FROM w ORDER BY a DESC",
  "keys.db|SELECT rowid FROM w",
  "keys.db|SELECT * FROM d",
  "keys.db|SELECT k, rowid FROM d",
  "generated.db|SELECT * FROM g",
  "generated.db|SELECT a, c FROM h",
  "generated.db|SELECT * FROM h ORDER BY c DESC",
  "small.db|SELECT * FROM t, t",
  "small.db|SELECT 1 UNION SELECT 2",
  "small.db|VALUES(1)",
  "small.db|SELECT 1",
  "small.db|SELECT 1 WHERE 0",
  "small.db|SELECT 'a' WHERE 1",
  "small.db|SELECT hex(d) FROM t",
  "small.db|SELECT a FROM t WHERE c > 0.0",
  "small.db|SELECT quote(c) FROM t",
  "small.db|SELECT main.t.a FROM t",
  "small.db|SELECT * FROM main.t",
  "small.db|SELECT z.a FROM t",
  "small.db|SELECT x",
  "small.db|SELECT a COLLATE NOCASE FROM t",
  "small.db|SELECT b FROM t ORDER BY b COLLATE NOCASE",
  "small.db|SELECT a FROM t ORDER BY 1, 1",
  "small.db|SELECT 1 FROM t ORDER BY 1",
  "small.db|SELECT a FROM t ORDER BY -1",
  "small.db|SELECT a FROM t ORDER BY b, a",
  "small.db|SELECT a FROM t WHERE a=1 ORDER BY a",
  "small.db|SELECT DISTINCT a>0 FROM t",
  "small.db|SELECT * FROM t WHERE 0",
  "small.db|SELECT a FROM t LIMIT 0",
  "small.db|SELECT a FROM t LIMIT 99",
  "small.db|SELECT a FROM t LIMIT 2 OFFSET 99",
  "small.db|SELECT t.rowid FROM t",
  "small.db|SELECT nosuchfn(a) FROM t",
  "small.db|SELECT * FROM t WHERE nosuch=1",
  "page512.db|SELECT n, s FROM wide WHERE n BETWEEN 100 AND 103",
  "page512.db|SELECT s FROM wide ORDER BY n DESC LIMIT 3",
  "page512.db|SELECT n FROM wide LIMIT 4",
  "page512.db|SELECT n FROM wide WHERE n%97=0",
  "indexed.db|SELECT a, b FROM k WHERE b='v50'",
  "indexed.db|SELECT a FROM k ORDER BY b LIMIT 5",
  "indexed.db|SELECT b FROM k WHERE a<4 ORDER BY a DESC",
  "overflow.db|SELECT length(t) FROM big",
  "overflow.db|SELECT substr(t,1,10) FROM big",
  "m-utf8-512.db|SELECT * FROM m",
  "m-utf8-1024.db|SELECT i, t FROM m WHERE r>2",
  "m-utf8-4096.db|SELECT * FROM m ORDER BY t",
  "m-utf8-65536.db|SELECT * FROM m",
  "m-reserved32.db|SELECT * FROM m",
  "m-wal.db|SELECT * FROM m",
  "m-autovacuum-full.db|SELECT * FROM m",
  "m-autovacuum-incr.db|SELECT * FROM m",
  "m-utf16le-512.db|SELECT * FROM m",
  "m-utf16be-4096.db|SELECT * FROM m",
  "utf16.db|SELECT * FROM u",
  "utf16.db|SELECT * FROM u ORDER BY t",
  "utf16.db|SELECT t, length(t), hex(t) FROM u",
  "utf16.db|SELECT t FROM u WHERE t = 'abc'",
  "utf16.db|SELECT typeof(t), t||'!' FROM u",
  "utf16.db|SELECT substr(t,2,1) FROM u",
  "utf16.db|SELECT DISTINCT t FROM u",
  "m-utf16le-4096.db|SELECT * FROM m ORDER BY t DESC",
  "m-utf16be-4096.db|SELECT t FROM m WHERE t > 'one' ORDER BY i",
  "m-utf16le-512.db|SELECT i, t, r, b FROM m ORDER BY t",
  "wide16.db|SELECT t FROM s ORDER BY t",
  "wide16.db|SELECT hex(t), length(t) FROM s",
  "wide16.db|SELECT t FROM s ORDER BY t DESC LIMIT 2",
  "wide16.db|SELECT t FROM s WHERE t > char(65533)",
  "wide16.db|SELECT t FROM n ORDER BY t",
  "wide16.db|SELECT t FROM n WHERE t='A' ORDER BY rowid",
  "m-utf16be-4096.db|SELECT hex(t), octet_length(t) FROM m ORDER BY i",
  "m-utf16be-4096.db|SELECT quote(CAST(t AS BLOB)) FROM m ORDER BY i",
  "m-utf16le-4096.db|SELECT hex(t), octet_length(t) FROM m ORDER BY i",
  "small.db|SELECT count(*), count(a), count(d) FROM t",
  "small.db|SELECT count(DISTINCT b), count(DISTINCT a>0) FROM t",
  "small.db|SELECT sum(a), total(a), avg(a) FROM t",
  "small.db|SELECT sum(c), total(c), avg(c) FROM t",
  "small.db|SELECT sum(b), sum(d), avg(b) FROM t",
  "small.db|SELECT sum(DISTINCT a), avg(DISTINCT a) FROM t",
  "small.db|SELECT min(a), max(a), min(b), max(b) FROM t",
  "small.db|SELECT min(d), max(d), min(c), max(c) FROM t",
  "small.db|SELECT max(a,b) FROM t",
  "small.db|SELECT group_concat(b), group_concat(a) FROM t",
  "small.db|SELECT group_concat(a,'-'), group_concat(b,NULL) FROM t",
  "small.db|SELECT string_agg(b,'/'), group_concat(DISTINCT b) FROM t",
  "small.db|SELECT hex(group_concat(d)), length(group_concat(d)) FROM t",
  "small.db|SELECT b, max(a) FROM t",
  "small.db|SELECT b, min(a) FROM t",
  "small.db|SELECT b, min(d) FROM t",
  "small.db|SELECT b, max(a), min(a) FROM t",
  "small.db|SELECT a, count(*) FROM t",
  "small.db|SELECT rowid, sum(a) FROM t",
  "small.db|SELECT *, count(*) FROM t",
  "small.db|SELECT count(*) FROM t WHERE a>5",
  "small.db|SELECT sum(a), avg(a), total(a), min(a), max(a) FROM t WHERE 0",
  "small.db|SELECT *, count(*) FROM t WHERE 0",
  "small.db|SELECT rowid, a, group_concat(b) FROM t WHERE 0",
  "small.db|SELECT typeof(sum(a)), typeof(total(a)), typeof(avg(a)) FROM t",
  "small.db|SELECT count(*)",
  "small.db|SELECT sum(9223372036854775807) FROM t",
  "small.db|SELECT total(9223372036854775807) FROM t",
  "small.db|SELECT avg(9223372036854775807) FROM t",
  "small.db|SELECT sum(CASE WHEN a<0 THEN 0.5 ELSE 9223372036854775807 END) FROM t",
  "small.db|SELECT sum(CASE WHEN a=1 THEN 0.5 ELSE a END) FROM t",
  "small.db|SELECT total(CASE WHEN a=1 THEN 0.5 ELSE a END) FROM t",
  "small.db|SELECT a>0, count(*) FROM t GROUP BY a>0",
  "small.db|SELECT a>0 AS p, count(*) FROM t GROUP BY p",
  "small.db|SELECT b, count(*) FROM t GROUP BY 1",
  "small.db|SELECT 'A', count(*) FROM t GROUP BY 2-1",
  "small.db|SELECT a, count(*) FROM t GROUP BY a ORDER BY a DESC",
  "small.db|SELECT a FROM t GROUP BY a HAVING count(*)=1",
  "small.db|SELECT sum(a) FROM t HAVING sum(a)>0",
  "small.db|SELECT sum(a) FROM t HAVING sum(a)<0",
  "small.db|SELECT count(*) FROM t GROUP BY d",
  "small.db|SELECT count(*), min(a) FROM t GROUP BY d ORDER BY 2",
  "small.db|SELECT a FROM t HAVING 1",
  "small.db|SELECT a FROM t WHERE count(*)>0",
  "small.db|SELECT count(count(*)) FROM t",
  "small.db|SELECT a FROM t GROUP BY count(*)",
  "small.db|SELECT '1', a FROM t ORDER BY count(*)",
  "small.db|SELECT group_concat(DISTINCT b, '-') FROM t",
  "small.db|SELECT count(DISTINCT a, b) FROM t",
  "small.db|SELECT a, count(*) FROM t GROUP BY 9",
  "small.db|SELECT sum(a) FROM t GROUP BY -1",
  "small.db|SELECT *, count(*) FROM t GROUP BY 1",
  "keys.db|SELECT count(*), sum(id), max(v) FROM r",
  "keys.db|SELECT v, max(id) FROM r",
  "keys.db|SELECT id, count(*) FROM r GROUP BY id ORDER BY id",
  "indexed.db|SELECT count(*), sum(a), avg(a) FROM k",
  "indexed.db|SELECT a%5, count(*) FROM k GROUP BY a%5",
  "indexed.db|SELECT count(*) FROM k WHERE a<=10 GROUP BY a%2",
  "page512.db|SELECT count(*), min(n), max(n), total(n) FROM wide",
  "page512.db|SELECT n%7, count(*), group_concat(n) FROM wide WHERE n<20 GROUP BY n%7",
  "wide16.db|SELECT count(DISTINCT t), group_concat(t) FROM n",
  "wide16.db|SELECT t, count(*) FROM n GROUP BY t ORDER BY t",
  "wide16.db|SELECT min(t), max(t) FROM n",
  "utf16.db|SELECT group_concat(t), max(t), length(group_concat(t)) FROM u",
  "m-utf16be-4096.db|SELECT t, max(i) FROM m GROUP BY i>1 ORDER BY 1",
  "m-utf8-4096.db|SELECT count(*), sum(i), group_concat(t,'|') FROM m",
  "overflow.db|SELECT count(*), length(group_concat(t)) FROM big",
  "small.db|SELECT sum(9e999), total(9e999), avg(9e999) FROM t",
  "small.db|SELECT sum(CASE WHEN a<0 THEN 0.5 ELSE -9223372036854775807 END) FROM t",
  "small.db|SELECT total(CASE WHEN a<0 THEN 0.5 ELSE -9223372036854775807 END) FROM t",
  "small.db|SELECT a>0 AS p, b AS q, count(*) FROM t GROUP BY q ORDER BY 1",
  "small.db|SELECT group_concat() FROM t",
  "small.db|SELECT sum(count(*)+1) FROM t",
  "small.db|SELECT count(*) FROM t WHERE nosuch=1",
  "small.db|SELECT count(nosuch) FROM t GROUP BY a",
  "small.db|SELECT DISTINCT count(*) FROM t GROUP BY a",
  "small.db|SELECT a, count(*) FROM t GROUP BY a ORDER BY count(*), a LIMIT 2",
  "keys.db|SELECT group_concat(v,'') FROM r GROUP BY id%2 ORDER BY 1",
  "small.db|SELECT a FROM t UNION SELECT 9",
  "small.db|SELECT a FROM t UNION ALL SELECT 9",
  "small.db|SELECT a FROM t EXCEPT SELECT 2",
  "small.db|SELECT a FROM t INTERSECT SELECT 2",
  "small.db|SELECT a FROM t UNION SELECT 9 ORDER BY 1 DESC",
  "small.db|SELECT a AS x FROM t UNION SELECT 9 ORDER BY x",
  "small.db|SELECT a FROM t INTERSECT SELECT 2 UNION SELECT 7",
  "small.db|SELECT a FROM t UNION ALL SELECT 9 LIMIT 2",
  "small.db|SELECT a FROM t UNION ALL SELECT 1 UNION SELECT 0",
  "small.db|SELECT a, b FROM t UNION SELECT 9, 'z' ORDER BY 2",
  "small.db|SELECT b FROM t UNION SELECT 9",
  "small.db|SELECT count(*) FROM t UNION SELECT 1",
  "small.db|SELECT 1, 2 UNION SELECT 3",
  "small.db|SELECT a FROM t UNION SELECT 9 ORDER BY a+1",
  "small.db|SELECT a FROM t UNION SELECT 9 ORDER BY 9",
  "small.db|VALUES(1),(2)",
  "small.db|VALUES(3,'a'),(1,'b')",
  "small.db|VALUES(1),(2,3)",
  "small.db|VALUES(1) UNION SELECT 2",
  "small.db|VALUES(2),(1) ORDER BY 1",
  "small.db|VALUES(a)",
  "small.db|SELECT * FROM temp.t",
  "small.db|SELECT temp.t.a FROM t",
  "small.db|SELECT main.t.a FROM main.t",
  "wide16.db|SELECT t FROM n UNION SELECT 'a'",
  "wide16.db|SELECT t FROM n UNION SELECT 'A'",
  "wide16.db|SELECT 'z' UNION SELECT t FROM n",
  "wide16.db|SELECT t FROM n WHERE rowid<=2 UNION SELECT t FROM n WHERE rowid=3",
  "wide16.db|SELECT t FROM n INTERSECT SELECT 'a'",
  "wide16.db|SELECT t FROM n EXCEPT SELECT 'zz'",
  "wide16.db|SELECT t FROM n UNION ALL SELECT 'a' ORDER BY 1",
  "wide16.db|SELECT DISTINCT t FROM n",
  "small.db|VALUES(1) LIMIT 1",
  "small.db|SELECT * FROM (SELECT 1)",
  "joins.db|SELECT * FROM a, b",
  "joins.db|SELECT * FROM a JOIN b ON a.x=b.x",
  "joins.db|SELECT * FROM a JOIN b USING(x)",
  "joins.db|SELECT * FROM a NATURAL JOIN b",
  "joins.db|SELECT * FROM a LEFT JOIN b ON a.x=b.x",
  "joins.db|SELECT * FROM a LEFT JOIN b USING(x)",
  "joins.db|SELECT * FROM a CROSS JOIN b",
  "joins.db|SELECT * FROM a INNER JOIN b ON a.x=b.x",
  "joins.db|SELECT a.x, b.z FROM a, b WHERE a.x=b.x",
  "joins.db|SELECT b.* FROM a JOIN b USING(x)",
  "joins.db|SELECT a.*, b.z FROM a JOIN b USING(x)",
  "joins.db|SELECT x FROM a JOIN b USING(x)",
  "joins.db|SELECT x FROM a, b",
  "joins.db|SELECT rowid FROM a, b",
  "joins.db|SELECT a.rowid, b.rowid FROM a, b LIMIT 3",
  "joins.db|SELECT * FROM a AS p, a AS q WHERE p.x<q.x",
  "joins.db|SELECT * FROM a, a",
  "joins.db|SELECT * FROM a JOIN b USING(y)",
  "joins.db|SELECT * FROM a NATURAL JOIN b ON a.x=b.x",
  "joins.db|SELECT * FROM a NATURAL JOIN c",
  "joins.db|SELECT * FROM a JOIN c USING(y)",
  "joins.db|SELECT * FROM c JOIN a USING(y)",
  "joins.db|SELECT * FROM a LEFT JOIN b ON a.x=b.x WHERE b.x IS NULL",
  "joins.db|SELECT * FROM a LEFT JOIN b ON 0",
  "joins.db|SELECT * FROM a JOIN b ON a.x=b.x JOIN c ON a.y=c.y",
  "joins.db|SELECT a.x, count(*) FROM a LEFT JOIN b ON a.x=b.x GROUP BY a.x ORDER BY a.x",
  "joins.db|SELECT count(*), max(b.z) FROM a JOIN b USING(x)",
  "joins.db|SELECT * FROM a RIGHT JOIN b ON a.x=b.x",
  "joins.db|SELECT * FROM a FULL JOIN b ON a.x=b.x",
  "joins.db|SELECT * FROM a, b ORDER BY 1, 2, 3, 4",
  "joins.db|SELECT * FROM a LEFT JOIN b USING(x) UNION SELECT 9,'z',9,'z'",
  "joins.db|SELECT DISTINCT a.x FROM a, b ORDER BY 1",
  "joins.db|SELECT * FROM a JOIN b",
  "joins.db|SELECT * FROM nosuch, a",
  "joins.db|SELECT * FROM a NATURAL JOIN b USING(x)",
  "joins.db|SELECT * FROM a JOIN b USING(z)",
  "small.db|SELECT *",
  "joins.db|SELECT * FROM a RIGHT JOIN b USING(x)",
  "joins.db|SELECT * FROM a FULL JOIN b USING(x)",
  "joins.db|SELECT a.x, b.x, x FROM a RIGHT JOIN b USING(x)",
  "joins.db|SELECT * FROM a RIGHT JOIN b ON 0",
  "joins.db|SELECT * FROM a FULL JOIN b ON 0",
  "joins.db|SELECT * FROM a RIGHT JOIN b ON a.x=b.x JOIN c ON b.z=c.y",
  "joins.db|SELECT * FROM a JOIN b ON a.x=b.x RIGHT JOIN c ON a.y=c.y",
  "joins.db|SELECT * FROM a RIGHT JOIN b ON a.x=b.x LEFT JOIN c ON a.y=c.y",
  "joins.db|SELECT * FROM a NATURAL RIGHT JOIN b",
  "joins.db|SELECT count(*), count(a.x) FROM a RIGHT JOIN b ON a.x=b.x",
  "joins.db|SELECT * FROM a RIGHT JOIN b ON a.x=b.x ORDER BY 3, 4",
  "joins.db|SELECT * FROM a FULL JOIN b ON a.x=b.x WHERE a.x IS NULL",
  "joins.db|SELECT * FROM a RIGHT JOIN b ON a.x=b.x WHERE b.x=4",
  "joins.db|SELECT abs(b.x) FROM a RIGHT JOIN b ON a.x=b.x",
  "joins.db|SELECT abs(b.x) FROM a JOIN b ON a.x=b.x",
  "generated.db|SELECT b, c FROM g ORDER BY a",
  "generated.db|SELECT * FROM g WHERE b>2",
  "generated.db|SELECT * FROM f",
  "generated.db|SELECT * FROM i",
  "generated.db|SELECT typeof(a), typeof(b), typeof(c) FROM i",
  "generated.db|SELECT count(*), sum(c), max(b) FROM g",
  "generated.db|SELECT * FROM g JOIN h ON g.c=h.c",
  "generated.db|SELECT * FROM g GROUP BY 2",
  "generated.db|SELECT rowid, * FROM f",
  "small.db|SELECT *, count(*) FROM t GROUP BY 2",
  "small.db|SELECT a, * FROM t GROUP BY 3 ORDER BY 1",
  "small.db|SELECT *, count(*) FROM t GROUP BY 9",
  "generated.db|SELECT * FROM j",
  "joins.db|SELECT a.*, count(*) FROM a GROUP BY 1 ORDER BY 1",
  "joins.db|SELECT *, count(*) FROM a JOIN b USING(x) GROUP BY 2 ORDER BY 1",
  "keys.db|SELECT * FROM u",
  "keys.db|SELECT * FROM p",
  "keys.db|SELECT * FROM q",
  "keys.db|SELECT typeof(a), c FROM q",
  "keys.db|SELECT rowid FROM w",
  "keys.db|SELECT count(*), max(a), min(c) FROM u",
  "keys.db|SELECT * FROM w JOIN u ON w.b=u.a",
  "keys.db|SELECT * FROM u WHERE c=3",
  "page512.db|SELECT count(*), min(k), max(k) FROM deep",
  "page512.db|SELECT k, v FROM deep WHERE k LIKE 'k1_' ORDER BY k",
  "page512.db|SELECT k FROM deep LIMIT 4",
  "page512.db|SELECT * FROM deep ORDER BY v DESC LIMIT 3",
  "small.db|WITH x AS (SELECT a FROM t) SELECT * FROM x",
  "small.db|WITH x(p) AS (SELECT a FROM t) SELECT p FROM x",
  "small.db|WITH x(p,q) AS (SELECT a FROM t) SELECT * FROM x",
  "small.db|WITH t(a) AS (SELECT 9) SELECT * FROM t",
  "small.db|WITH t(a) AS (SELECT 9) SELECT * FROM main.t",
  "small.db|WITH x AS (SELECT a, b FROM t) SELECT x.a FROM x WHERE x.a>0",
  "small.db|WITH x AS (SELECT 1 AS p) SELECT * FROM (SELECT p FROM x)",
  "small.db|WITH x AS (SELECT 1) SELECT * FROM x, x",
  "small.db|SELECT s.a FROM (SELECT a FROM t) AS s",
  "small.db|SELECT * FROM (SELECT a FROM t) WHERE a='1'",
  "small.db|SELECT rowid FROM (SELECT a FROM t)",
  "small.db|SELECT * FROM (SELECT 1), (SELECT 2)",
  "small.db|SELECT * FROM (SELECT * FROM (SELECT 1))",
  "small.db|SELECT * FROM (VALUES(1,2))",
  "small.db|SELECT * FROM (SELECT a FROM t UNION SELECT 9) ORDER BY 1",
  "small.db|SELECT a, count(*) FROM (SELECT a FROM t) GROUP BY a ORDER BY 1",
  "small.db|SELECT x.* FROM t",
  "small.db|SELECT * FROM nosuch(1)",
  "joins.db|SELECT a.* FROM a, a",
  "joins.db|SELECT * FROM a JOIN (SELECT x, z FROM b) USING(x)",
  "joins.db|SELECT * FROM a LEFT JOIN (SELECT x, z FROM b) AS s ON a.x=s.x",
  "joins.db|SELECT * FROM a NATURAL JOIN (SELECT x, z FROM b)",
  "keys.db|SELECT * FROM (SELECT a, b FROM w)",
  "wide16.db|SELECT * FROM (SELECT t FROM n) WHERE t='A'",
  "wide16.db|SELECT DISTINCT t FROM (SELECT t FROM n)",
  "joins.db|SELECT a.x FROM a, (SELECT 1) ORDER BY 1",
  "small.db|SELECT (SELECT max(a) FROM t)",
  "small.db|SELECT (SELECT 1), (SELECT 2)",
  "small.db|SELECT (SELECT a FROM t WHERE a>99)",
  "small.db|SELECT (SELECT a, b FROM t)",
  "small.db|SELECT a, (SELECT count(*) FROM t AS u WHERE u.a<t.a) FROM t",
  "small.db|SELECT a FROM t WHERE (SELECT count(*) FROM t AS u WHERE u.a<t.a)=1",
  "small.db|SELECT a FROM t WHERE EXISTS (SELECT 1 FROM t AS u WHERE u.a=t.a+1)",
  "small.db|SELECT a FROM t WHERE NOT EXISTS (SELECT 1 FROM t AS u WHERE u.a=t.a+1) ORDER BY 1",
  "small.db|SELECT EXISTS (SELECT a, b FROM t)",
  "small.db|SELECT EXISTS (SELECT a FROM t WHERE 0)",
  "small.db|SELECT a FROM t WHERE a IN (SELECT a FROM t WHERE a>1)",
  "small.db|SELECT a FROM t WHERE a NOT IN (SELECT a FROM t WHERE a>1) ORDER BY 1",
  "small.db|SELECT a FROM t WHERE a IN (SELECT 1 UNION SELECT 2) ORDER BY 1",
  "small.db|SELECT NULL IN (SELECT a FROM t)",
  "small.db|SELECT NULL IN (SELECT a FROM t WHERE 0)",
  "small.db|SELECT 1 IN (SELECT a FROM t WHERE 0)",
  "small.db|SELECT 1 NOT IN (SELECT a FROM t WHERE 0)",
  "small.db|SELECT a FROM t WHERE a IN (SELECT a, b FROM t)",
  "small.db|SELECT a FROM t WHERE a IN t",
  "small.db|SELECT a FROM t WHERE a IN nosuch",
  "small.db|SELECT a FROM t WHERE a IN temp.t",
  "small.db|VALUES((SELECT 1))",
  "small.db|SELECT a FROM t LIMIT (SELECT 2)",
  "small.db|SELECT a FROM t UNION SELECT 9 LIMIT (SELECT 2)",
  "small.db|WITH x AS (SELECT 7 AS p) SELECT (SELECT p FROM x)",
  "small.db|SELECT * FROM (SELECT (SELECT max(a) FROM t) AS m)",
  "small.db|SELECT a FROM t WHERE a IN (SELECT max(a) FROM t GROUP BY b) ORDER BY 1",
  "joins.db|SELECT x FROM a WHERE x IN (SELECT x FROM b)",
  "joins.db|SELECT x FROM a WHERE x NOT IN (SELECT x FROM b)",
  "joins.db|SELECT x FROM a WHERE x IN b",
  "wide16.db|SELECT 'A' IN (SELECT t FROM n)",
  "wide16.db|SELECT t FROM n WHERE t IN (SELECT 'a') ORDER BY 1",
  "keys.db|SELECT a FROM w WHERE a IN (SELECT a FROM w)",
  "wide16.db|SELECT t FROM n WHERE t IN s ORDER BY 1",
  "wide16.db|SELECT t FROM n WHERE t NOT IN s ORDER BY 1",
  "wide16.db|SELECT t FROM s WHERE t IN n ORDER BY 1",
  "small.db|SELECT a FROM t WHERE rowid=2",
  "small.db|SELECT a FROM t WHERE 2=rowid",
  "small.db|SELECT a FROM t WHERE oid=2",
  "small.db|SELECT a FROM t WHERE _rowid_=2",
  "small.db|SELECT a FROM t WHERE rowid>1 ORDER BY 1",
  "small.db|SELECT a FROM t WHERE rowid>=2 ORDER BY 1",
  "small.db|SELECT a FROM t WHERE rowid<2",
  "small.db|SELECT a FROM t WHERE rowid<=1",
  "small.db|SELECT a FROM t WHERE rowid>1 AND rowid<3",
  "small.db|SELECT a FROM t WHERE rowid>1 AND rowid<1",
  "small.db|SELECT a FROM t WHERE rowid=0",
  "small.db|SELECT a FROM t WHERE rowid=-1",
  "small.db|SELECT a FROM t WHERE rowid=9223372036854775807",
  "small.db|SELECT a FROM t WHERE rowid>9223372036854775807",
  "small.db|SELECT a FROM t WHERE rowid<-9223372036854775808",
  "small.db|SELECT a FROM t WHERE rowid=1.0",
  "small.db|SELECT a FROM t WHERE rowid='2'",
  "small.db|SELECT a FROM t WHERE rowid<>2 ORDER BY 1",
  "small.db|SELECT a FROM t WHERE rowid=2 OR rowid=3 ORDER BY 1",
  "small.db|SELECT a FROM t WHERE NOT (rowid=2) ORDER BY 1",
  "small.db|SELECT a FROM t WHERE rowid=2 AND a=2",
  "small.db|SELECT a FROM t WHERE a=2 AND rowid=2",
  "small.db|SELECT a FROM t WHERE main.t.rowid=2",
  "small.db|SELECT a FROM t WHERE temp.t.rowid=2",
  "small.db|SELECT a FROM t AS x WHERE x.rowid=2",
  "small.db|SELECT a FROM t AS x WHERE y.rowid=2",
  "small.db|SELECT a FROM t WHERE rowid=(SELECT 2)",
  "keys.db|SELECT * FROM r WHERE id=5",
  "keys.db|SELECT * FROM r WHERE id>2 ORDER BY 1",
  "keys.db|SELECT * FROM r WHERE rowid=9",
  "keys.db|SELECT * FROM d WHERE k=1",
  "keys.db|SELECT * FROM w WHERE a='x'",
  "keys.db|SELECT rowid FROM w WHERE rowid=1",
  "joins.db|SELECT * FROM a, b WHERE a.rowid=1 AND b.rowid=2",
  "joins.db|SELECT * FROM a LEFT JOIN b ON a.x=b.x WHERE b.rowid=1",
  "joins.db|SELECT * FROM a LEFT JOIN b ON a.x=b.x WHERE a.rowid=3",
  "joins.db|SELECT * FROM a RIGHT JOIN b ON a.x=b.x WHERE b.rowid<=2 ORDER BY 3",
  "joins.db|SELECT * FROM a AS p, a AS q WHERE rowid=1",
  "joins.db|SELECT * FROM a JOIN b ON a.rowid=b.rowid",
  "page512.db|SELECT n FROM wide WHERE rowid=399",
  "page512.db|SELECT n FROM wide WHERE rowid>=398 ORDER BY 1",
  "page512.db|SELECT count(*) FROM wide WHERE rowid>200 AND rowid<=205",
  "page512.db|SELECT k FROM deep WHERE rowid=1",
  "small.db|SELECT a FROM t WHERE 2>=rowid ORDER BY 1",
  "small.db|SELECT a FROM t WHERE 2>rowid",
  "small.db|SELECT a FROM t WHERE 2<rowid",
  "small.db|SELECT a FROM t WHERE 2<=rowid ORDER BY 1",
  "small.db|SELECT a FROM t WHERE rowid>1 AND rowid>2",
  "small.db|SELECT a FROM t WHERE rowid>2 AND rowid>1",
  "small.db|SELECT a FROM t WHERE rowid<3 AND rowid<2",
  "small.db|SELECT a FROM t WHERE rowid<2 AND rowid<3",
  "small.db|SELECT a FROM t WHERE rowid=2 AND rowid=2",
  "small.db|SELECT a FROM t WHERE rowid=2 AND rowid=3",
  "indexed.db|SELECT b FROM k WHERE a=77",
  "indexed.db|SELECT a FROM k WHERE b='v77'",
  "indexed.db|SELECT a FROM k WHERE b='V77'",
  "indexed.db|SELECT a FROM k WHERE a='77'",
  "indexed.db|SELECT a FROM k WHERE a=77.0",
  "indexed.db|SELECT a FROM k WHERE a=NULL",
  "indexed.db|SELECT a FROM k WHERE b IS NULL",
  "indexed.db|SELECT a FROM k WHERE a=101",
  "indexed.db|SELECT count(*) FROM k WHERE a>50",
  "indexed.db|SELECT a FROM k WHERE a=1 AND b='v1'",
  "indexed.db|SELECT a FROM k WHERE a=1 OR a=2 ORDER BY 1",
  "indexed.db|SELECT * FROM m WHERE p=1 ORDER BY 1,2",
  "indexed.db|SELECT * FROM m WHERE p=2 ORDER BY 1,2",
  "indexed.db|SELECT * FROM m WHERE q='a' ORDER BY 1,2",
  "indexed.db|SELECT * FROM m WHERE q='A' ORDER BY 1,2",
  "indexed.db|SELECT * FROM m WHERE q IS NULL",
  "indexed.db|SELECT * FROM m WHERE r=10",
  "indexed.db|SELECT * FROM m WHERE r=50",
  "indexed.db|SELECT * FROM m WHERE p=1 AND q='b'",
  "indexed.db|SELECT * FROM e WHERE a=-1",
  "indexed.db|SELECT * FROM e WHERE b=1",
  "indexed.db|SELECT * FROM e WHERE b=3",
  "indexed.db|SELECT a FROM o WHERE t='short'",
  "indexed.db|SELECT * FROM u WHERE b=1",
  "indexed.db|SELECT * FROM u WHERE a='y'",
  "indexed.db|SELECT k.a, m.r FROM k JOIN m ON k.a=m.p WHERE k.a=2 ORDER BY 2",
  "indexed.db|SELECT * FROM m AS x, m AS y WHERE x.p=1 AND y.p=3 ORDER BY 2,5",
  "indexed.db|SELECT * FROM m LEFT JOIN k ON m.p=k.a WHERE m.q='A' ORDER BY 1,2",
  "indexed.db|SELECT * FROM m WHERE p=(SELECT 1) ORDER BY 2",
  "indexed.db|SELECT * FROM m WHERE 1=p ORDER BY 2",
  "indexed.db|SELECT * FROM m WHERE m.p=1 ORDER BY 2",
  "indexed.db|SELECT * FROM m WHERE main.m.p=1 ORDER BY 2",
  "indexed.db|SELECT * FROM m AS z WHERE z.p=1 ORDER BY 2",
  "indexed.db|SELECT * FROM m WHERE p=1 AND rowid=1",
  "indexed.db|SELECT * FROM m WHERE r=x'10'",
  "indexed.db|SELECT * FROM m WHERE q=3",
  "page512.db|SELECT n FROM wide WHERE s='row 399'",
  "page512.db|SELECT n FROM wide WHERE s='row 1'",
  "page512.db|SELECT n FROM wide WHERE s='row 200'",
  "page512.db|SELECT n FROM wide WHERE s='nothing'",
  "page512.db|SELECT n FROM wide WHERE s='zzz'",
  "page512.db|SELECT count(*) FROM wide WHERE s='row 5' OR s='row 6'",
  "indexed.db|SELECT rowid FROM f WHERE v=1.5",
  "indexed.db|SELECT rowid FROM f WHERE v=7",
  "indexed.db|SELECT rowid FROM f WHERE v=x'0102'",
  "indexed.db|SELECT rowid FROM f WHERE v='t'",
  "indexed.db|SELECT rowid FROM f WHERE v IS NULL",
  "indexed.db|SELECT typeof(v) FROM f ORDER BY rowid",
  /* The configuration matrix of document 16, every statement under
  ** every configuration, which is section 16.11. */
  "m-utf8-512.db|SELECT count(*) FROM m",
  "m-utf8-512.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-utf8-512.db|SELECT t FROM m ORDER BY t DESC",
  "m-utf8-512.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-utf8-512.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-utf8-512.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-utf8-512.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-utf8-512.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-utf8-512.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-utf8-512.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-utf8-512.db|SELECT i FROM m WHERE t='two'",
  "m-utf8-512.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-utf8-512.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-utf8-512.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-utf8-512.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-utf8-512.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-utf8-512.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-utf8-1024.db|SELECT count(*) FROM m",
  "m-utf8-1024.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-utf8-1024.db|SELECT t FROM m ORDER BY t DESC",
  "m-utf8-1024.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-utf8-1024.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-utf8-1024.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-utf8-1024.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-utf8-1024.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-utf8-1024.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-utf8-1024.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-utf8-1024.db|SELECT i FROM m WHERE t='two'",
  "m-utf8-1024.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-utf8-1024.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-utf8-1024.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-utf8-1024.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-utf8-1024.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-utf8-1024.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-utf8-4096.db|SELECT count(*) FROM m",
  "m-utf8-4096.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-utf8-4096.db|SELECT t FROM m ORDER BY t DESC",
  "m-utf8-4096.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-utf8-4096.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-utf8-4096.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-utf8-4096.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-utf8-4096.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-utf8-4096.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-utf8-4096.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-utf8-4096.db|SELECT i FROM m WHERE t='two'",
  "m-utf8-4096.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-utf8-4096.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-utf8-4096.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-utf8-4096.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-utf8-4096.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-utf8-4096.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-utf8-8192.db|SELECT count(*) FROM m",
  "m-utf8-8192.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-utf8-8192.db|SELECT t FROM m ORDER BY t DESC",
  "m-utf8-8192.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-utf8-8192.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-utf8-8192.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-utf8-8192.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-utf8-8192.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-utf8-8192.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-utf8-8192.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-utf8-8192.db|SELECT i FROM m WHERE t='two'",
  "m-utf8-8192.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-utf8-8192.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-utf8-8192.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-utf8-8192.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-utf8-8192.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-utf8-8192.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-utf8-65536.db|SELECT count(*) FROM m",
  "m-utf8-65536.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-utf8-65536.db|SELECT t FROM m ORDER BY t DESC",
  "m-utf8-65536.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-utf8-65536.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-utf8-65536.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-utf8-65536.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-utf8-65536.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-utf8-65536.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-utf8-65536.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-utf8-65536.db|SELECT i FROM m WHERE t='two'",
  "m-utf8-65536.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-utf8-65536.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-utf8-65536.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-utf8-65536.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-utf8-65536.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-utf8-65536.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-utf16le-512.db|SELECT count(*) FROM m",
  "m-utf16le-512.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-utf16le-512.db|SELECT t FROM m ORDER BY t DESC",
  "m-utf16le-512.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-utf16le-512.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-utf16le-512.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-utf16le-512.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-utf16le-512.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-utf16le-512.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-utf16le-512.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-utf16le-512.db|SELECT i FROM m WHERE t='two'",
  "m-utf16le-512.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-utf16le-512.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-utf16le-512.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-utf16le-512.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-utf16le-512.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-utf16le-512.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-utf16le-4096.db|SELECT count(*) FROM m",
  "m-utf16le-4096.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-utf16le-4096.db|SELECT t FROM m ORDER BY t DESC",
  "m-utf16le-4096.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-utf16le-4096.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-utf16le-4096.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-utf16le-4096.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-utf16le-4096.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-utf16le-4096.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-utf16le-4096.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-utf16le-4096.db|SELECT i FROM m WHERE t='two'",
  "m-utf16le-4096.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-utf16le-4096.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-utf16le-4096.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-utf16le-4096.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-utf16le-4096.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-utf16le-4096.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-utf16be-512.db|SELECT count(*) FROM m",
  "m-utf16be-512.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-utf16be-512.db|SELECT t FROM m ORDER BY t DESC",
  "m-utf16be-512.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-utf16be-512.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-utf16be-512.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-utf16be-512.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-utf16be-512.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-utf16be-512.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-utf16be-512.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-utf16be-512.db|SELECT i FROM m WHERE t='two'",
  "m-utf16be-512.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-utf16be-512.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-utf16be-512.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-utf16be-512.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-utf16be-512.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-utf16be-512.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-utf16be-4096.db|SELECT count(*) FROM m",
  "m-utf16be-4096.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-utf16be-4096.db|SELECT t FROM m ORDER BY t DESC",
  "m-utf16be-4096.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-utf16be-4096.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-utf16be-4096.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-utf16be-4096.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-utf16be-4096.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-utf16be-4096.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-utf16be-4096.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-utf16be-4096.db|SELECT i FROM m WHERE t='two'",
  "m-utf16be-4096.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-utf16be-4096.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-utf16be-4096.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-utf16be-4096.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-utf16be-4096.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-utf16be-4096.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-reserved4.db|SELECT count(*) FROM m",
  "m-reserved4.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-reserved4.db|SELECT t FROM m ORDER BY t DESC",
  "m-reserved4.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-reserved4.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-reserved4.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-reserved4.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-reserved4.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-reserved4.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-reserved4.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-reserved4.db|SELECT i FROM m WHERE t='two'",
  "m-reserved4.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-reserved4.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-reserved4.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-reserved4.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-reserved4.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-reserved4.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-reserved32.db|SELECT count(*) FROM m",
  "m-reserved32.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-reserved32.db|SELECT t FROM m ORDER BY t DESC",
  "m-reserved32.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-reserved32.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-reserved32.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-reserved32.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-reserved32.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-reserved32.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-reserved32.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-reserved32.db|SELECT i FROM m WHERE t='two'",
  "m-reserved32.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-reserved32.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-reserved32.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-reserved32.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-reserved32.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-reserved32.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-wal.db|SELECT count(*) FROM m",
  "m-wal.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-wal.db|SELECT t FROM m ORDER BY t DESC",
  "m-wal.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-wal.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-wal.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-wal.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-wal.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-wal.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-wal.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-wal.db|SELECT i FROM m WHERE t='two'",
  "m-wal.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-wal.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-wal.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-wal.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-wal.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-wal.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-autovacuum-full.db|SELECT count(*) FROM m",
  "m-autovacuum-full.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-autovacuum-full.db|SELECT t FROM m ORDER BY t DESC",
  "m-autovacuum-full.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-autovacuum-full.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-autovacuum-full.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-autovacuum-full.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-autovacuum-full.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-autovacuum-full.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-autovacuum-full.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-autovacuum-full.db|SELECT i FROM m WHERE t='two'",
  "m-autovacuum-full.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-autovacuum-full.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-autovacuum-full.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-autovacuum-full.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-autovacuum-full.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-autovacuum-full.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-autovacuum-incr.db|SELECT count(*) FROM m",
  "m-autovacuum-incr.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-autovacuum-incr.db|SELECT t FROM m ORDER BY t DESC",
  "m-autovacuum-incr.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-autovacuum-incr.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-autovacuum-incr.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-autovacuum-incr.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-autovacuum-incr.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-autovacuum-incr.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-autovacuum-incr.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-autovacuum-incr.db|SELECT i FROM m WHERE t='two'",
  "m-autovacuum-incr.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-autovacuum-incr.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-autovacuum-incr.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-autovacuum-incr.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-autovacuum-incr.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-autovacuum-incr.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-delete.db|SELECT count(*) FROM m",
  "m-delete.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-delete.db|SELECT t FROM m ORDER BY t DESC",
  "m-delete.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-delete.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-delete.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-delete.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-delete.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-delete.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-delete.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-delete.db|SELECT i FROM m WHERE t='two'",
  "m-delete.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-delete.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-delete.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-delete.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-delete.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-delete.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-truncate.db|SELECT count(*) FROM m",
  "m-truncate.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-truncate.db|SELECT t FROM m ORDER BY t DESC",
  "m-truncate.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-truncate.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-truncate.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-truncate.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-truncate.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-truncate.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-truncate.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-truncate.db|SELECT i FROM m WHERE t='two'",
  "m-truncate.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-truncate.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-truncate.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-truncate.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-truncate.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-truncate.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-persist.db|SELECT count(*) FROM m",
  "m-persist.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-persist.db|SELECT t FROM m ORDER BY t DESC",
  "m-persist.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-persist.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-persist.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-persist.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-persist.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-persist.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-persist.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-persist.db|SELECT i FROM m WHERE t='two'",
  "m-persist.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-persist.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-persist.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-persist.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-persist.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-persist.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-memory.db|SELECT count(*) FROM m",
  "m-memory.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-memory.db|SELECT t FROM m ORDER BY t DESC",
  "m-memory.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-memory.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-memory.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-memory.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-memory.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-memory.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-memory.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-memory.db|SELECT i FROM m WHERE t='two'",
  "m-memory.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-memory.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-memory.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-memory.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-memory.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-memory.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1",
  "m-off.db|SELECT count(*) FROM m",
  "m-off.db|SELECT i, t, r, quote(b) FROM m ORDER BY i",
  "m-off.db|SELECT t FROM m ORDER BY t DESC",
  "m-off.db|SELECT typeof(i), typeof(t), typeof(r), typeof(b) FROM m WHERE i=1",
  "m-off.db|SELECT length(t), unicode(t) FROM m ORDER BY i",
  "m-off.db|SELECT upper(t), lower(t) FROM m WHERE i=2",
  "m-off.db|SELECT sum(i), avg(r), min(t), max(t), group_concat(t,'-') FROM m",
  "m-off.db|SELECT t, count(*) FROM m GROUP BY t ORDER BY t",
  "m-off.db|SELECT rowid, i FROM m WHERE rowid=2",
  "m-off.db|SELECT i FROM m WHERE rowid>1 ORDER BY i",
  "m-off.db|SELECT i FROM m WHERE t='two'",
  "m-off.db|SELECT i FROM m WHERE t>'one' ORDER BY i",
  "m-off.db|SELECT a.i, b.i FROM m AS a JOIN m AS b ON a.i=b.i-1 ORDER BY 1",
  "m-off.db|SELECT i, (SELECT count(*) FROM m AS u WHERE u.i<m.i) FROM m ORDER BY i",
  "m-off.db|SELECT * FROM (SELECT t FROM m WHERE i>1) ORDER BY 1",
  "m-off.db|SELECT i FROM m WHERE i IN (SELECT i FROM m WHERE r>2.0) ORDER BY 1",
  "m-off.db|SELECT octet_length(t), hex(t), CAST(t AS BLOB)=x'6f6e65' FROM m WHERE i=1"
};

/* The cases, one per line. */
static void query_corpus(void) {
  int i;
  for (i = 0; i < (int)(sizeof(aQuery) / sizeof(aQuery[0])); i++) {
    printf("%s\n", aQuery[i]);
  }
}

/* A database written under the schema format the file format document
** calls 1: `SQLITE_DBCONFIG_LEGACY_FILE_FORMAT` is what asks for it,
** and no pragma the shell takes does, which is why this is here and not
** in `sqlite-fixtures.sh`. Every statement after the first argument is
** run in order, so ALTER TABLE ADD COLUMN raises the format to 2 or 3
** from here. Answers the format the file ends with. */
static int legacy_case(int argc, char **argv) {
  sqlite3 *db = 0;
  int rc, i;
  unsigned char aHdr[48];
  FILE *f;
  remove(argv[2]);
  rc = sqlite3_open(argv[2], &db);
  if (rc == SQLITE_OK) {
    rc = sqlite3_db_config(db, SQLITE_DBCONFIG_LEGACY_FILE_FORMAT, 1, 0);
  }
  for (i = 3; rc == SQLITE_OK && i < argc; i++) {
    char *zErr = 0;
    rc = sqlite3_exec(db, argv[i], 0, 0, &zErr);
    if (rc != SQLITE_OK) {
      fprintf(stderr, "legacy: %s\n", zErr ? zErr : sqlite3_errmsg(db));
      sqlite3_free(zErr);
    }
  }
  if (rc != SQLITE_OK) {
    sqlite3_close(db);
    return 1;
  }
  sqlite3_close(db);
  f = fopen(argv[2], "rb");
  if (f == 0 || fread(aHdr, 1, sizeof(aHdr), f) != sizeof(aHdr)) {
    fprintf(stderr, "legacy: cannot read %s\n", argv[2]);
    if (f) fclose(f);
    return 1;
  }
  fclose(f);
  printf("schema format %u\n",
         (aHdr[44] << 24) | (aHdr[45] << 16) | (aHdr[46] << 8) | aHdr[47]);
  return 0;
}

/* The rows one statement makes of one file. */
static int query_case(const char *line, const char *zDir) {
  const char *bar = strchr(line, '|');
  char *path, *sql;
  sqlite3 *db = 0;
  sqlite3_stmt *stmt = 0;
  int rc, i, n;
  if (bar == 0) return 1;
  path = sqlite3_mprintf("%s/%.*s", zDir, (int)(bar - line), line);
  sql = sqlite3_mprintf("%s", bar + 1);
  if (path == 0 || sql == 0) return 1;
  rc = sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, 0);
  if (rc == SQLITE_OK) rc = sqlite3_prepare_v2(db, sql, -1, &stmt, 0);
  if (rc != SQLITE_OK) {
    printf("!\t%s\n", sqlite3_errmsg(db));
    sqlite3_finalize(stmt);
    sqlite3_close(db);
    sqlite3_free(path);
    sqlite3_free(sql);
    return 0;
  }
  n = sqlite3_column_count(stmt);
  printf("N");
  for (i = 0; i < n; i++) printf("|%s", sqlite3_column_name(stmt, i));
  while ((rc = sqlite3_step(stmt)) == SQLITE_ROW) {
    printf("\tR");
    for (i = 0; i < n; i++) {
      sqlite3_str *str = sqlite3_str_new(db);
      char *text;
          sqlite3QuoteValue(str, sqlite3_column_value(stmt, i), 0);
      text = sqlite3_str_finish(str);
      printf("|%s", text ? text : "");
      sqlite3_free(text);
    }
  }
  if (rc != SQLITE_DONE) printf("\t!%s", sqlite3_errmsg(db));
  printf("\n");
  sqlite3_finalize(stmt);
  sqlite3_close(db);
  sqlite3_free(path);
  sqlite3_free(sql);
  return 0;
}

/* The magic a rollback journal begins with, from `aJournalMagic` of
** `src/pager.c`. */
static const unsigned char aJournalMagic[] = {
  0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7
};

/* The sector a journal header is padded to. SQLite assumes 512 where the
** device does not say otherwise, which is what `setSectorSize` does. */
#define JOURNAL_SECTOR 512

/* Writes a 32-bit value big-endian, which every field of a journal is. */
static void put32(unsigned char *p, unsigned int v) {
  p[0] = (unsigned char)(v >> 24);
  p[1] = (unsigned char)(v >> 16);
  p[2] = (unsigned char)(v >> 8);
  p[3] = (unsigned char)(v);
}

/* The checksum of one page record, which is `pager_cksum` of
** `src/pager.c`: the nonce plus every two-hundredth byte counting back
** from the end of the page. */
static unsigned int journal_cksum(unsigned int init, const unsigned char *aData,
                                  int pageSize) {
  unsigned int cksum = init;
  int i = pageSize - 200;
  while (i > 0) {
    cksum += aData[i];
    i -= 200;
  }
  return cksum;
}

/* Reads a whole file, or answers zero. */
static unsigned char *slurp(const char *zName, long *pLen) {
  FILE *f = fopen(zName, "rb");
  unsigned char *p;
  long n;
  if (f == 0) return 0;
  fseek(f, 0, SEEK_END);
  n = ftell(f);
  fseek(f, 0, SEEK_SET);
  p = (unsigned char *)malloc((size_t)(n > 0 ? n : 1));
  if (p == 0 || fread(p, 1, (size_t)n, f) != (size_t)n) {
    free(p);
    fclose(f);
    return 0;
  }
  fclose(f);
  *pLen = n;
  return p;
}

/* Writes the hot rollback journal that turns `zNew` back into `zOld`.
**
** A journal holds the content each page had before the transaction, so
** the records are the pages of `zOld` that `zNew` does not match, and
** the header's page count is how many pages `zOld` has. SQLite writes
** such a journal only between the sync of its records and the sync of
** the database, which a crash has to land inside; building it here is
** what makes that state a fixture. The pair is checked afterwards by
** opening it with the C library, which rolls it back. */
static int journal_case(const char *zOld, const char *zNew, const char *zOut) {
  long nOld = 0, nNew = 0;
  unsigned char *aOld = slurp(zOld, &nOld);
  unsigned char *aNew = slurp(zNew, &nNew);
  unsigned char aHdr[JOURNAL_SECTOR];
  unsigned int init = 0x5eed1234u;
  int pageSize, pages, nRec = 0, i;
  FILE *f;
  if (aOld == 0 || aNew == 0) {
    fprintf(stderr, "journal: cannot read %s or %s\n", zOld, zNew);
    return 1;
  }
  pageSize = (aOld[16] << 8) | aOld[17];
  if (pageSize == 1) pageSize = 65536;
  pages = (int)(nOld / pageSize);
  for (i = 1; i <= pages; i++) {
    long at = (long)(i - 1) * pageSize;
    if (at + pageSize > nNew || memcmp(aOld + at, aNew + at, (size_t)pageSize) != 0) {
      nRec++;
    }
  }
  memset(aHdr, 0, sizeof(aHdr));
  memcpy(aHdr, aJournalMagic, sizeof(aJournalMagic));
  put32(aHdr + 8, (unsigned int)nRec);
  put32(aHdr + 12, init);
  put32(aHdr + 16, (unsigned int)pages);
  put32(aHdr + 20, JOURNAL_SECTOR);
  put32(aHdr + 24, (unsigned int)pageSize);
  f = fopen(zOut, "wb");
  if (f == 0) {
    fprintf(stderr, "journal: cannot write %s\n", zOut);
    return 1;
  }
  fwrite(aHdr, 1, sizeof(aHdr), f);
  for (i = 1; i <= pages; i++) {
    long at = (long)(i - 1) * pageSize;
    unsigned char aNum[4], aSum[4];
    if (at + pageSize <= nNew && memcmp(aOld + at, aNew + at, (size_t)pageSize) == 0) {
      continue;
    }
    put32(aNum, (unsigned int)i);
    put32(aSum, journal_cksum(init, aOld + at, pageSize));
    fwrite(aNum, 1, 4, f);
    fwrite(aOld + at, 1, (size_t)pageSize, f);
    fwrite(aSum, 1, 4, f);
  }
  fclose(f);
  free(aOld);
  free(aNew);
  printf("%d records over %d pages of %d bytes\n", nRec, pages, pageSize);
  return 0;
}

int main(int argc, char **argv) {
  char line[4096];
  if (argc < 2) {
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
  if (strcmp(argv[1], "legacy") == 0) {
    if (argc < 4) {
      fprintf(stderr, "legacy: usage: legacy <path> <sql>...\n");
      return 2;
    }
    return legacy_case(argc, argv);
  }
  if (strcmp(argv[1], "query-corpus") == 0) {
    query_corpus();
    return 0;
  }
  if (strcmp(argv[1], "query") == 0) {
    const char *zDir = argc > 2 ? argv[2] : ".";
    while (fgets(line, sizeof(line), stdin) != 0) {
      size_t len = strlen(line);
      while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r')) {
        line[--len] = 0;
      }
      if (len == 0) continue;
      if (query_case(line, zDir) != 0) return 1;
    }
    return 0;
  }
  if (strcmp(argv[1], "journal") == 0) {
    if (argc < 5) {
      fprintf(stderr, "usage: sqlite-oracle journal <old.db> <new.db> <out>\n");
      return 2;
    }
    return journal_case(argv[2], argv[3], argv[4]);
  }
  if (strcmp(argv[1], "schema-corpus") == 0) {
    schema_corpus();
    return 0;
  }
  if (strcmp(argv[1], "schema") == 0) {
    while (fgets(line, sizeof(line), stdin) != 0) {
      size_t len = strlen(line);
      while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r')) {
        line[--len] = 0;
      }
      if (len == 0) continue;
      if (schema_case(line) != 0) return 1;
    }
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
