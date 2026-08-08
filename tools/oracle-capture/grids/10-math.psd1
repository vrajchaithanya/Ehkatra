# Aggregation and arithmetic. docs/32 compat catalogue: SUM accumulation order,
# the skip-versus-coerce split (a range skips text and logicals, a literal
# argument coerces them), and IEEE overflow reported as #NUM! rather than Inf.
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'math'

    # Shared fixture, A1:H40. Every reference below resolves against this.
    fixture = @(
        # A: clean ascending numbers
        @{ ref = 'A1'; value = 1 }
        @{ ref = 'A2'; value = 2 }
        @{ ref = 'A3'; value = 3 }
        @{ ref = 'A4'; value = 4 }
        @{ ref = 'A5'; value = 5 }
        # B: the mixed column -- number, blank, text-that-looks-numeric, logical, negative
        @{ ref = 'B1'; value = 10 }
        @{ ref = 'B2'; blank = $true }
        @{ ref = 'B3'; text  = '7' }
        @{ ref = 'B4'; value = $true }
        @{ ref = 'B5'; value = -2 }
        # C: entirely blank
        @{ ref = 'C1'; blank = $true }
        @{ ref = 'C2'; blank = $true }
        @{ ref = 'C3'; blank = $true }
        # D: an error inside a range
        @{ ref = 'D1'; formula = '=NA()' }
        @{ ref = 'D2'; value   = 10 }
        @{ ref = 'D3'; formula = '=1/0' }
        # E: non-numeric text beside numbers
        @{ ref = 'E1'; value = 'abc' }
        @{ ref = 'E2'; value = 2.5 }
        @{ ref = 'E3'; value = 0 }
        # F: the extremes of the double range. These MUST be seeded through
        # Value2 rather than written as formula literals: Excel's formula parser
        # rejects any literal at or beyond 1E+308 outright, and truncates
        # anything at or below 1E-308 to zero. A cell value has no such limit.
        @{ ref = 'F1'; value = 1E+308 }
        @{ ref = 'F2'; value = 1E+308 }
        @{ ref = 'F4'; value = 1.7976931348623157E+308 }
        @{ ref = 'F5'; value = 5E-324 }
        # G: the cancellation triple, and a formula-produced empty string
        @{ ref = 'G1'; value = 0.1 }
        @{ ref = 'G2'; value = 0.2 }
        @{ ref = 'G3'; value = -0.3 }
        @{ ref = 'G4'; formula = '=""' }
        # H: text that Excel's coercion rules treat specially
        @{ ref = 'H1'; text = '1E2' }
        @{ ref = 'H2'; text = ' 5 ' }
        @{ ref = 'H3'; text = '$1,234.50' }
        @{ ref = 'H4'; text = '50%' }
    )

    functions = @(
        @{
            name = 'SUM'
            doc  = 'Ranges skip text and logicals; literal arguments coerce them. Accumulation is left to right, so the order of the addends is observable.'
            cases = @(
                @{ formula = '=SUM(1,2,3)';            tags = @('basic') }
                @{ formula = '=SUM(A1:A5)';            tags = @('basic', 'range') }
                @{ formula = '=SUM(C1:C3)';            tags = @('blank'); note = 'all-blank range' }
                @{ formula = '=SUM(B1:B5)';            tags = @('coercion', 'range'); note = 'text "7" and TRUE are skipped, not coerced' }
                @{ formula = '=SUM("7",1)';            tags = @('coercion', 'literal'); note = 'a literal text argument IS coerced' }
                @{ formula = '=SUM(TRUE,1)';           tags = @('coercion', 'literal') }
                @{ formula = '=SUM(TRUE,FALSE)';       tags = @('coercion', 'literal') }
                @{ formula = '=SUM("abc",1)';          tags = @('error-input') }
                @{ formula = '=SUM(D1:D2)';            tags = @('error-input'); note = '#N/A inside the range propagates' }
                @{ formula = '=SUM(D2:D3)';            tags = @('error-input'); note = '#DIV/0! inside the range propagates' }
                @{ formula = '=SUM(E1:E3)';            tags = @('coercion', 'range') }
                @{ formula = '=SUM(F1:F2)';            tags = @('boundary', 'overflow'); note = 'IEEE would give +Inf; Excel reports an error' }
                @{ formula = '=SUM(G1:G3)';            tags = @('compat-bug', 'precision'); note = 'D-041 compat_final_adjust: catastrophic cancellation' }
                @{ formula = '=SUM(0.1,0.2)';          tags = @('precision') }
                @{ formula = '=SUM(A1:A5,B1)';         tags = @('range') }
                @{ formula = '=SUM(H1:H4)';            tags = @('coercion', 'range'); note = 'numeric-looking text in a range' }
                @{ formula = '=SUM(-0)';               tags = @('boundary') }
                @{ formula = '=SUM(1E16,1,-1E16)';     tags = @('compat-bug', 'precision', 'accumulation') }
                @{ formula = '=SUM(-1E16,1,1E16)';     tags = @('compat-bug', 'precision', 'accumulation'); note = 'same addends, different order' }
                @{ formula = '=SUM(0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1)'; tags = @('precision', 'accumulation') }
            )
        }

        @{
            name = 'PRODUCT'
            doc  = 'Empty PRODUCT is 0 in Excel, not the multiplicative identity.'
            cases = @(
                @{ formula = '=PRODUCT(2,3,4)';   tags = @('basic') }
                @{ formula = '=PRODUCT(A1:A5)';   tags = @('range') }
                @{ formula = '=PRODUCT(C1:C3)';   tags = @('blank', 'boundary'); note = 'no numeric cells at all' }
                @{ formula = '=PRODUCT(B1:B5)';   tags = @('coercion', 'range') }
                @{ formula = '=PRODUCT("3",4)';   tags = @('coercion', 'literal') }
                @{ formula = '=PRODUCT(0,5)';     tags = @('boundary') }
                @{ formula = '=PRODUCT(E1:E3)';   tags = @('coercion', 'range'); note = 'E3 is 0' }
                @{ formula = '=PRODUCT(F1:F2)';   tags = @('overflow', 'boundary') }
                @{ formula = '=PRODUCT(D1:D2)';   tags = @('error-input') }
                @{ formula = '=PRODUCT(-2,-3)';   tags = @('basic') }
            )
        }

        @{
            name = 'AVERAGE'
            doc  = 'Divides by the count of numeric cells only, so an empty selection is #DIV/0! rather than 0.'
            cases = @(
                @{ formula = '=AVERAGE(1,2,3,4)';  tags = @('basic') }
                @{ formula = '=AVERAGE(A1:A5)';    tags = @('range') }
                @{ formula = '=AVERAGE(C1:C3)';    tags = @('blank', 'boundary'); note = 'no numbers to divide by' }
                @{ formula = '=AVERAGE(B1:B5)';    tags = @('coercion', 'range'); note = 'denominator counts only B1 and B5' }
                @{ formula = '=AVERAGE(E1:E3)';    tags = @('coercion', 'range') }
                @{ formula = '=AVERAGE("2",4)';    tags = @('coercion', 'literal') }
                @{ formula = '=AVERAGE(TRUE,1)';   tags = @('coercion', 'literal') }
                @{ formula = '=AVERAGE(D1:D2)';    tags = @('error-input') }
                @{ formula = '=AVERAGE(1,2)';      tags = @('precision') }
                @{ formula = '=AVERAGE(1,2,2)';    tags = @('precision'); note = '5/3 is not representable' }
                @{ formula = '=AVERAGE(G1:G3)';    tags = @('precision', 'compat-bug') }
            )
        }

        @{
            name = 'COUNT'
            doc  = 'Counts numbers. A logical or numeric text inside a range does not count; the same value as a literal argument does. That asymmetry is the whole test.'
            cases = @(
                @{ formula = '=COUNT(A1:A5)';    tags = @('basic') }
                @{ formula = '=COUNT(B1:B5)';    tags = @('coercion', 'range'); note = 'B3 text and B4 TRUE are not numbers here' }
                @{ formula = '=COUNT(TRUE)';     tags = @('coercion', 'literal'); note = 'a literal logical does count' }
                @{ formula = '=COUNT("7")';      tags = @('coercion', 'literal') }
                @{ formula = '=COUNT("abc")';    tags = @('coercion', 'literal') }
                @{ formula = '=COUNT(C1:C3)';    tags = @('blank') }
                @{ formula = '=COUNT(D1:D2)';    tags = @('error-input'); note = 'COUNT tolerates errors in a range' }
                @{ formula = '=COUNT(G4)';       tags = @('blank'); note = 'a formula returning "" is text, not a number' }
                @{ formula = '=COUNT(E1:E3)';    tags = @('range') }
                @{ formula = '=COUNT(A1:A5,B1:B5)'; tags = @('range') }
            )
        }

        @{
            name = 'COUNTA'
            doc  = 'Counts non-empty cells, so it disagrees with COUNT exactly where coercion would have mattered.'
            cases = @(
                @{ formula = '=COUNTA(A1:A5)';   tags = @('basic') }
                @{ formula = '=COUNTA(B1:B5)';   tags = @('range'); note = 'B2 is the only empty one' }
                @{ formula = '=COUNTA(C1:C3)';   tags = @('blank') }
                @{ formula = '=COUNTA(D1:D3)';   tags = @('error-input'); note = 'error cells are non-empty' }
                @{ formula = '=COUNTA(G4)';      tags = @('boundary'); note = 'a formula returning "" counts as non-empty' }
                @{ formula = '=COUNTA("")';      tags = @('literal', 'boundary'); note = 'a literal empty string' }
                @{ formula = '=COUNTA(E1:E3)';   tags = @('range') }
                @{ formula = '=COUNTA(A1:A5,C1:C3)'; tags = @('range') }
            )
        }

        @{
            name = 'COUNTBLANK'
            doc  = 'The one aggregation that treats a formula-produced empty string as blank, unlike COUNTA and ISBLANK.'
            cases = @(
                @{ formula = '=COUNTBLANK(C1:C3)';  tags = @('basic') }
                @{ formula = '=COUNTBLANK(B1:B5)';  tags = @('range') }
                @{ formula = '=COUNTBLANK(A1:A5)';  tags = @('range') }
                @{ formula = '=COUNTBLANK(G4)';     tags = @('compat-bug', 'boundary'); note = 'the =""-is-blank quirk; ISBLANK says FALSE for the same cell' }
                @{ formula = '=COUNTBLANK(D1:D3)';  tags = @('error-input') }
                @{ formula = '=COUNTBLANK(G1:G4)';  tags = @('range') }
            )
        }

        @{
            name = 'MIN'
            doc  = 'Returns 0 rather than an error when nothing numeric is in range.'
            cases = @(
                @{ formula = '=MIN(A1:A5)';    tags = @('basic') }
                @{ formula = '=MIN(C1:C3)';    tags = @('blank', 'boundary'); note = 'Excel answers 0, not #NUM!' }
                @{ formula = '=MIN(B1:B5)';    tags = @('coercion', 'range') }
                @{ formula = '=MIN("3",5)';    tags = @('coercion', 'literal') }
                @{ formula = '=MIN(TRUE,5)';   tags = @('coercion', 'literal') }
                @{ formula = '=MIN(E1:E3)';    tags = @('range') }
                @{ formula = '=MIN(D1:D2)';    tags = @('error-input') }
                @{ formula = '=MIN(-0.0,0)';   tags = @('boundary') }
                @{ formula = '=MIN(G1:G3)';    tags = @('range') }
            )
        }

        @{
            name = 'MAX'
            doc  = 'Mirror of MIN, including the empty-selection zero.'
            cases = @(
                @{ formula = '=MAX(A1:A5)';    tags = @('basic') }
                @{ formula = '=MAX(C1:C3)';    tags = @('blank', 'boundary') }
                @{ formula = '=MAX(B1:B5)';    tags = @('coercion', 'range') }
                @{ formula = '=MAX("9",5)';    tags = @('coercion', 'literal') }
                @{ formula = '=MAX(TRUE,0)';   tags = @('coercion', 'literal') }
                @{ formula = '=MAX(E1:E3)';    tags = @('range') }
                @{ formula = '=MAX(D1:D2)';    tags = @('error-input') }
                @{ formula = '=MAX(F1:F5)';    tags = @('boundary'); note = 'spans the whole magnitude range' }
                @{ formula = '=MAX(-5,-3)';    tags = @('basic') }
            )
        }

        @{
            name = 'ABS'
            doc  = 'Single-argument coercion probe: blank, logical, numeric text, non-numeric text, error.'
            cases = @(
                @{ formula = '=ABS(-3)';       tags = @('basic') }
                @{ formula = '=ABS(3)';        tags = @('basic') }
                @{ formula = '=ABS(0)';        tags = @('boundary') }
                @{ formula = '=ABS(-0.0)';     tags = @('boundary'); note = 'negative zero' }
                @{ formula = '=ABS(C1)';       tags = @('blank'); note = 'blank coerces to 0' }
                @{ formula = '=ABS(TRUE)';     tags = @('coercion') }
                @{ formula = '=ABS("-3")';     tags = @('coercion') }
                @{ formula = '=ABS(H1)';       tags = @('coercion'); note = 'text "1E2" -- the gene-symbol mangling case' }
                @{ formula = '=ABS(H2)';       tags = @('coercion'); note = 'text " 5 " with surrounding spaces' }
                @{ formula = '=ABS(H3)';       tags = @('coercion'); note = 'currency-formatted text' }
                @{ formula = '=ABS(H4)';       tags = @('coercion'); note = 'percent text' }
                @{ formula = '=ABS(E1)';       tags = @('error-input') }
                @{ formula = '=ABS(D1)';       tags = @('error-input') }
                @{ formula = '=ABS(-F1)';      tags = @('boundary'); note = 'the extreme comes from a fixture cell; Excel rejects the equivalent literal' }
                @{ formula = '=ABS(-F4)';      tags = @('boundary'); note = 'the largest finite double' }
            )
        }

        @{
            name = 'SIGN'
            cases = @(
                @{ formula = '=SIGN(5)';       tags = @('basic') }
                @{ formula = '=SIGN(-5)';      tags = @('basic') }
                @{ formula = '=SIGN(0)';       tags = @('boundary') }
                @{ formula = '=SIGN(-0.0)';    tags = @('boundary') }
                @{ formula = '=SIGN(C1)';      tags = @('blank') }
                @{ formula = '=SIGN(F5)';      tags = @('boundary'); note = 'a subnormal seeded via Value2 -- kept, or flattened to zero' }
                @{ formula = '=SIGN(1E-300)';  tags = @('boundary'); note = 'a small exponent the formula parser still accepts' }
                @{ formula = '=SIGN(UNICHAR(8722)&"1")'; tags = @('coercion'); note = 'U+2212 minus sign, not ASCII hyphen. Built with UNICHAR so this grid file stays pure ASCII and cannot be mangled by an encoding guess.' }
                @{ formula = '=SIGN("-1")';    tags = @('coercion') }
                @{ formula = '=SIGN(E1)';      tags = @('error-input') }
            )
        }

        @{
            name = 'SQRT'
            cases = @(
                @{ formula = '=SQRT(4)';        tags = @('basic') }
                @{ formula = '=SQRT(2)';        tags = @('precision') }
                @{ formula = '=SQRT(0)';        tags = @('boundary') }
                @{ formula = '=SQRT(-1)';       tags = @('error-input', 'boundary') }
                @{ formula = '=SQRT(-0.0)';     tags = @('boundary'); note = 'does negative zero count as negative?' }
                @{ formula = '=SQRT(C1)';       tags = @('blank') }
                @{ formula = '=SQRT("4")';      tags = @('coercion') }
                @{ formula = '=SQRT(TRUE)';     tags = @('coercion') }
                @{ formula = '=SQRT(E1)';       tags = @('error-input') }
                @{ formula = '=SQRT(F1)';       tags = @('boundary') }
                @{ formula = '=SQRT(F4)';       tags = @('boundary') }
                @{ formula = '=SQRT(F5)';       tags = @('boundary', 'precision') }
            )
        }

        @{
            name = 'MOD'
            doc  = 'Takes the sign of the divisor, unlike a C remainder. Fractional operands are allowed.'
            cases = @(
                @{ formula = '=MOD(5,3)';        tags = @('basic') }
                @{ formula = '=MOD(-5,3)';       tags = @('negative'); note = 'sign follows the divisor' }
                @{ formula = '=MOD(5,-3)';       tags = @('negative') }
                @{ formula = '=MOD(-5,-3)';      tags = @('negative') }
                @{ formula = '=MOD(5,0)';        tags = @('error-input', 'boundary') }
                @{ formula = '=MOD(0,5)';        tags = @('boundary') }
                @{ formula = '=MOD(5.5,2)';      tags = @('basic') }
                @{ formula = '=MOD(-5.5,2)';     tags = @('negative') }
                @{ formula = '=MOD(5,C1)';       tags = @('blank'); note = 'blank divisor is 0' }
                @{ formula = '=MOD("7","3")';    tags = @('coercion') }
                @{ formula = '=MOD(1E+15,3)';    tags = @('precision', 'boundary') }
                @{ formula = '=MOD(1E+16,3)';    tags = @('precision', 'boundary'); note = 'past exact-integer range for a double' }
                @{ formula = '=MOD(E1,3)';       tags = @('error-input') }
            )
        }

        @{
            name = 'POWER'
            cases = @(
                @{ formula = '=POWER(2,10)';     tags = @('basic') }
                @{ formula = '=POWER(2,0)';      tags = @('boundary') }
                @{ formula = '=POWER(0,0)';      tags = @('boundary'); note = 'indeterminate in maths; Excel picks a side' }
                @{ formula = '=POWER(0,-1)';     tags = @('error-input', 'boundary') }
                @{ formula = '=POWER(2,-1)';     tags = @('basic') }
                @{ formula = '=POWER(-8,1/3)';   tags = @('error-input', 'boundary'); note = 'real cube root of a negative' }
                @{ formula = '=POWER(-2,2)';     tags = @('negative') }
                @{ formula = '=POWER(-2,3)';     tags = @('negative') }
                @{ formula = '=POWER(10,400)';   tags = @('overflow', 'boundary') }
                @{ formula = '=POWER(10,-400)';  tags = @('boundary'); note = 'underflow to zero or an error?' }
                @{ formula = '=POWER(1.0000001,10000000)'; tags = @('precision') }
                @{ formula = '=POWER("2","3")';  tags = @('coercion') }
                @{ formula = '=POWER(C1,C2)';    tags = @('blank', 'boundary'); note = 'blank^blank' }
                @{ formula = '=POWER(2,0.5)';    tags = @('precision') }
            )
        }

        @{
            name = 'INT'
            doc  = 'Floors toward negative infinity, which is not truncation.'
            cases = @(
                @{ formula = '=INT(1.9)';       tags = @('basic') }
                @{ formula = '=INT(-1.5)';      tags = @('negative'); note = 'floor, not truncate' }
                @{ formula = '=INT(-1.0)';      tags = @('boundary') }
                @{ formula = '=INT(0)';         tags = @('boundary') }
                @{ formula = '=INT(-0.5)';      tags = @('negative', 'boundary') }
                @{ formula = '=INT("2.7")';     tags = @('coercion') }
                @{ formula = '=INT(C1)';        tags = @('blank') }
                @{ formula = '=INT(TRUE)';      tags = @('coercion') }
                @{ formula = '=INT(F1)';        tags = @('boundary') }
                @{ formula = '=INT(F4)';        tags = @('boundary'); note = 'flooring the largest finite double' }
                @{ formula = '=INT(2.5)';       tags = @('boundary') }
                @{ formula = '=INT(E1)';        tags = @('error-input') }
            )
        }
    )
}
