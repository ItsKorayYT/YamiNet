cmake_minimum_required(VERSION 3.18)
project(YamiIntelQvl LANGUAGES CXX)
# Compile Intel's unmodified official QVL implementation against system OpenSSL.
# No SGX enclave or Intel-signed QvE is claimed or required for wallet-local QVL.
find_package(OpenSSL 3 REQUIRED)
set(S "${INTEL_QVL_SOURCE}/Src")
if(NOT EXISTS "${S}/AttestationLibrary/include/SgxEcdsaAttestation/QuoteVerification.h")
  message(FATAL_ERROR "INTEL_QVL_SOURCE must point to the pinned official Intel QVL checkout")
endif()
set(VERSION "1.0.0")
foreach(component AttestationLibrary AttestationParsers)
  configure_file("${S}/${component}/templates/Version.h.in" "${S}/${component}/src/Version/Version.h" @ONLY)
endforeach()
file(GLOB LIB_SOURCES "${S}/AttestationLibrary/src/*.cpp" "${S}/AttestationLibrary/src/*/*.cpp" "${S}/AttestationLibrary/src/*/*/*.cpp")
file(GLOB PARSER_SOURCES "${S}/AttestationParsers/src/*.cpp" "${S}/AttestationParsers/src/*/*.cpp")
file(GLOB COMMON_SOURCES "${S}/AttestationCommons/src/*.cpp" "${S}/AttestationCommons/src/Utils/*.cpp")
# The explicit globs include core verifiers/parsers; no test-generated quotes or app code.
add_library(yami_intel_qvl SHARED abi_check.cpp ${LIB_SOURCES} ${PARSER_SOURCES} ${COMMON_SOURCES})
target_compile_features(yami_intel_qvl PRIVATE cxx_std_17)
target_include_directories(yami_intel_qvl PRIVATE
 "${S}/AttestationLibrary/include" "${S}/AttestationLibrary/src"
 "${S}/AttestationParsers/include" "${S}/AttestationParsers/src"
 "${S}/AttestationCommons/include" "${S}/AttestationCommons/include/Utils"
 "${S}/ThirdParty/rapidjson/include")
target_link_libraries(yami_intel_qvl PRIVATE OpenSSL::Crypto OpenSSL::SSL)
