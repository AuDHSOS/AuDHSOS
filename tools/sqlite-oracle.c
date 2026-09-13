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

int main(int argc, char **argv) {
  char line[4096];
  if (argc != 2) {
    fprintf(stderr, "usage: sqlite-oracle fp-corpus|fp|num-corpus|num\n");
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
