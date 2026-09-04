# test-support

The property-test engine and the model-test runner used by every host test
in the workspace. Generators produce values together with their shrink
candidates (integrated shrinking), so `map` and `filter` need no inverse
functions. Runs are deterministic: the seed derives from the test name and
can be overridden with `AUDHSOS_PROPTEST_SEED` to replay a failure.
