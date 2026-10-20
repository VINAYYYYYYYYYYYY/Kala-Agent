# GPT-6.1-Sol review — parsing repairs required

Independently ran cargo test --release --locked:55passed, including85fixture cases, live differential and allocation guard. fmt/clippy -D warnings/release clean. Inspected native Decimal parsing/comparison/context, duration parsing/gridding and budget input surface. Acceptance withheld because additional live-Python/native probes found these concrete gaps.

## P2 — valid Unicode numeric inputs rejected

Python Decimal accepts Unicode decimal digits (Nd) in mantissa AND exponent; RunBudget(max_cost=...) accepts them too. Examples: ArabicIndic "١.٢٣" ->1.23; fullwidth "１２.３４" ->12.34; supplementary mathematical digits "𝟙𝟚.𝟛" ->12.3; "1e٢" ->1E+2. Rust Decimal::parse rejects all InvalidSyntax. Decimal also accepts surrounding ASCII separators U+001C..U+001F through Python whitespace semantics, while char::is_whitespace() does not include them; U+001C1U+001C accepted by Python but rejected here.

Python float("١.٢٣"),float("１２.３４"),float("1e٢") also accept; python_float_parse rejects each. Preserve exact accepted Unicode decimal digits, not broad numeric chars (superscripts/fractions/Roman numerals must not be accepted merely because numeric). Decimal and float underscore/whitespace semantics differ; do not normalize all strings with one permissive rule. Use representative live differential across Unicode Nd blocks, nonNd numeric lookalikes, whitespace, signs/underscores/exponents and JSON RunBudget/state entry paths.

## P2 — construction exponent bounds differ from CPython

Live oracle decimal.MAX_EMAX=999999999999999999, MIN_ETINY=-1999999999999999997.
- Rust accepts 1e-9223372036854775808 and0e-9223372036854775808; Python Decimal raises InvalidOperation.
- Rust accepts1e-1999999999999999998; Python rejects. Exact MIN_ETINY boundary1e-1999999999999999997 is valid.
- Rust accepts99e999999999999999999, rendering9.9E+1000000000000000000; Python rejects (adjusted exponent too large). Neighbor99e999999999999999998 is valid.
Current parser checks positive raw exponent only and permits negative i64 bounds. Validate effective exponent/adjusted exponent and coefficient-dependent limits exactly against live Python, including zero, leading/trailing zeros, fractional coefficient shifts and huge written exponents with harmless leading zeros. Keep arbitrary coefficient length/scale and bounded far-exponent arithmetic; no powers-of-ten allocation at these enormous exponents. Native from_parts/Context constructors are trusted typed APIs but must not introduce panics in accepted production paths; document any unvalidated typed API preconditions clearly.

## Integration cautions / nonblocking tracked limitations

State serde output order/float text is not canonical persisted hash bytes. Later core integration must convert values losslessly to reviewed kala-json and preserve int-vs-float. Do not use serde serialization as state digest bytes. Pydantic multiline error text remains a future wrapper compatibility gap; no full contract parity claim. Default-context add/sub is the required budget behavior, not a complete Decimal library.

Budget contains its own product python_float_repr. Now foundation is reviewed, prefer reuse kala-json::format_float rather than maintaining a second copy; a path dependency within your crate is allowed without parent workspace edits. Keep original public wrapper API if used by tests/callers and preserve invalid-value behavior. No native Windows execution/performance comparison claimed.
