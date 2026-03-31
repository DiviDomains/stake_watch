---
name: test_before_deploy
description: Always test changes before deploying - verify API responses and JS behavior locally
type: feedback
---

ALWAYS test changes before asking the user to test them. Public explorer endpoints don't need auth.

**Why:** The user was shown broken pages multiple times (NaN amounts, empty charts, wrong data) because changes were deployed without verifying the API response format matches what the JS expects.

**How to apply:**
1. After editing API code: `curl` the endpoint and verify the JSON response
2. After editing JS: test with `node -e` that parsing logic works with actual API data
3. Check field names match between Rust structs and JS (e.g. `amount_divi` vs `amount`)
4. Only deploy after both API and JS are verified
