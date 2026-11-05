// Reviewed Rust FFI ABI contract, compiled against the pinned OFFICIAL Intel header.
// No verification implementation is replaced or mocked here.
#include <SgxEcdsaAttestation/QuoteVerification.h>
#include <type_traits>
static_assert(sizeof(int) == 4, "Rust c_int requires 32-bit native int");
static_assert(sizeof(Status) == sizeof(int), "Rust c_int must match Intel Status");
static_assert(sizeof(time_t) == 8, "This adapter supports 64-bit Linux time_t only");
using SizeFn = Status (*)(const uint8_t*, uint32_t, uint32_t*);
using DataFn = Status (*)(const uint8_t*, uint32_t, uint32_t, uint8_t*, uint16_t*);
using PckFn = Status (*)(const char*, const char* const*, const char*, const time_t*);
using CollateralFn = Status (*)(const char*, const char*, const char*, const char*, const time_t*);
using QuoteFn = Status (*)(const uint8_t*, uint32_t, const char*, const char*, const char*, const char*);
static_assert(std::is_same_v<decltype(&sgxAttestationGetQECertificationDataSize), SizeFn>);
static_assert(std::is_same_v<decltype(&sgxAttestationGetQECertificationData), DataFn>);
static_assert(std::is_same_v<decltype(&sgxAttestationVerifyPCKCertificate), PckFn>);
static_assert(std::is_same_v<decltype(&sgxAttestationVerifyTCBInfo), CollateralFn>);
static_assert(std::is_same_v<decltype(&sgxAttestationVerifyEnclaveIdentity), CollateralFn>);
static_assert(std::is_same_v<decltype(&sgxAttestationVerifyQuote), QuoteFn>);
