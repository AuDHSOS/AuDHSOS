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
*/
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "sqlite3.h"

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

int main(int argc, char **argv) {
  char line[4096];
  if (argc != 2) {
    fprintf(stderr, "usage: sqlite-oracle fp-corpus | fp < cases\n");
    return 2;
  }
  if (strcmp(argv[1], "fp-corpus") == 0) {
    fp_corpus();
    return 0;
  }
  if (strcmp(argv[1], "fp") != 0) {
    fprintf(stderr, "sqlite-oracle: unknown mode %s\n", argv[1]);
    return 2;
  }
  while (fgets(line, sizeof(line), stdin) != 0) {
    size_t len = strlen(line);
    while (len > 0 && (line[len - 1] == '\n' || line[len - 1] == '\r')) {
      line[--len] = 0;
    }
    if (len == 0) continue;
    if (fp_case(line) != 0) {
      fprintf(stderr, "sqlite-oracle: bad case %s\n", line);
      return 1;
    }
  }
  return 0;
}
