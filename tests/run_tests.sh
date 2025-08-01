#!/bin/bash

# GX Language Test Runner
# Runs all test categories and reports results

set -e

echo "🧪 GX Language Test Suite"
echo "========================="
echo ""

# Configuration
TEST_DIR="tests"
BIN_DIR="bin"
RESULTS_DIR="test_results"
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# Create results directory
mkdir -p $RESULTS_DIR

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test counter
increment_total() {
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
}

increment_passed() {
    PASSED_TESTS=$((PASSED_TESTS + 1))
}

increment_failed() {
    FAILED_TESTS=$((FAILED_TESTS + 1))
}

# Test runner function
run_test() {
    local test_file=$1
    local test_name=$2
    
    echo -e "${BLUE}Running: ${test_name}${NC}"
    
    increment_total
    
    if [ -f "$test_file" ]; then
        if ./$BIN_DIR/gx "$test_file" > "$RESULTS_DIR/${test_name}.log" 2>&1; then
            echo -e "  ${GREEN}✅ PASSED${NC}"
            increment_passed
        else
            echo -e "  ${RED}❌ FAILED${NC}"
            increment_failed
            echo -e "  ${YELLOW}Check log: $RESULTS_DIR/${test_name}.log${NC}"
        fi
    else
        echo -e "  ${RED}❌ TEST FILE NOT FOUND${NC}"
        increment_failed
    fi
    
    echo ""
}

# Run all test categories
echo "🔧 Running Brain Process Tests..."
run_test "tests/test_brain_processes.gx" "brain_processes"

echo "🔧 Running Compilation Tests..."
run_test "tests/test_compilation.gx" "compilation"

echo "🔧 Running Distributed Tests..."
run_test "tests/test_distributed.gx" "distributed"

echo "🔧 Running Optimization Tests..."
run_test "tests/test_optimization.gx" "optimization"

echo "🔧 Running Memory Tests..."
run_test "tests/test_memory.gx" "memory"

echo "🔧 Running Message Tests..."
run_test "tests/test_messages.gx" "messages"

echo "🔧 Running Helper Tests..."
run_test "tests/test_helpers.gx" "helpers"

echo "🔧 Running Recipe Tests..."
run_test "tests/test_recipes.gx" "recipes"

echo "🔧 Running Objective Tests..."
run_test "tests/test_objectives.gx" "objectives"

echo "🔧 Running Integration Tests..."
run_test "tests/test_integration.gx" "integration"

# Performance tests
echo "🔧 Running Performance Tests..."
run_test "tests/test_performance.gx" "performance"

# Example tests
echo "🔧 Running Example Tests..."
run_test "examples/hello_world.gx" "hello_world_example"
run_test "examples/calculator.gx" "calculator_example"
run_test "examples/data_processor.gx" "data_processor_example"

# Generate test report
echo "📊 Test Results Summary"
echo "======================"
echo "Total Tests: $TOTAL_TESTS"
echo -e "Passed: ${GREEN}$PASSED_TESTS${NC}"
echo -e "Failed: ${RED}$FAILED_TESTS${NC}"

if [ $FAILED_TESTS -eq 0 ]; then
    echo -e "\n${GREEN}🎉 All tests passed!${NC}"
    exit 0
else
    echo -e "\n${RED}❌ Some tests failed. Check logs in $RESULTS_DIR/${NC}"
    exit 1
fi 