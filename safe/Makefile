include ../Makefile

LIB_NAME=serum_safe_program
TEST_PROGRAM_ID=91DN77tXQ4k5FFedLZWGQrqRmAtfThC7sMFWzEArWo2D
TEST_WHITELIST_PROGRAM_ID=EszpReFoTa8tTT5CoSH4JMDhvVzTLMoNPBUU13GqxrcT

.PHONY: test deploy-whitelist-program test-progra%

#
# Safe tests need to deploy *both* the safe and test-whitelist-program
# before running.
#
test: deploy-whitelist-program deploy-super test-progra%
	@ # no-op

deploy-whitelist-program:
	$(eval TMP=$(shell make -s -C ../wl deploy))
	$(eval TEST_WHITELIST_PROGRAM_ID=$(shell echo $(TMP) | sed 's/.*{programId: \(.*\)}.*/\1/g'))

test-progra%:
	RUST_BACKTRACE=1 \
	TEST_PROGRAM_ID=$(TEST_PROGRAM_ID) \
	TEST_PAYER_FILEPATH=$(TEST_PAYER_FILEPATH) \
	TEST_CLUSTER=$(TEST_CLUSTER) \
	TEST_WHITELIST_PROGRAM_ID=$(TEST_WHITELIST_PROGRAM_ID) \
	cargo test --features test,client -- --nocapture $(args)
