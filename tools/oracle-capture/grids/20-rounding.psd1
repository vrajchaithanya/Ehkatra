# Rounding. The family where Excel is least like IEEE 754 and least like any
# language's standard library: ROUND is half-away-from-zero applied to the
# *decimal* reading of the operand, so =ROUND(2.675,2) does not do what the
# binary value 2.67499999999999982236431605997495353221893310546875 implies.
# CEILING and FLOOR disagree with each other about a zero significance, which is
# the kind of asymmetry only the binary can tell you about.
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'rounding'

    fixture = @(
        @{ ref = 'A1'; blank = $true }
        @{ ref = 'A2'; value = 2.5 }
        @{ ref = 'A3'; value = -2.5 }
        @{ ref = 'A4'; text  = '2.5' }
        @{ ref = 'A5'; formula = '=NA()' }
        @{ ref = 'B1'; value = 1.7976931348623157E+308 }
    )

    functions = @(
        @{
            name = 'ROUND'
            doc  = 'Half away from zero, on the decimal reading of the operand rather than its binary value. Negative digits round to the left of the point.'
            cases = @(
                @{ formula = '=ROUND(2.5,0)';        tags = @('basic', 'half'); note = 'half away from zero, not half to even' }
                @{ formula = '=ROUND(-2.5,0)';       tags = @('negative', 'half') }
                @{ formula = '=ROUND(1.5,0)';        tags = @('half') }
                @{ formula = '=ROUND(0.5,0)';        tags = @('half', 'boundary') }
                @{ formula = '=ROUND(-0.5,0)';       tags = @('half', 'boundary') }
                @{ formula = '=ROUND(2.675,2)';      tags = @('precision', 'compat-bug'); note = 'the binary value is below 2.675; does Excel still round up?' }
                @{ formula = '=ROUND(1.005,2)';      tags = @('precision', 'compat-bug'); note = 'binary value is below 1.005' }
                @{ formula = '=ROUND(2.665,2)';      tags = @('precision', 'compat-bug'); note = 'binary value is above 2.665' }
                @{ formula = '=ROUND(1.0049999999,2)'; tags = @('precision') }
                @{ formula = '=ROUND(1234.5678,-2)'; tags = @('basic'); note = 'negative digits' }
                @{ formula = '=ROUND(1234.5678,-4)'; tags = @('boundary') }
                @{ formula = '=ROUND(0.12345,10)';   tags = @('boundary'); note = 'more digits than the value has' }
                @{ formula = '=ROUND(1.23456789,309)'; tags = @('boundary'); note = 'digits beyond the exponent range' }
                @{ formula = '=ROUND(2.5,A1)';       tags = @('blank'); note = 'blank digit count is 0' }
                @{ formula = '=ROUND(A4,0)';         tags = @('coercion'); note = 'numeric-looking text in a cell' }
                @{ formula = '=ROUND("2.5",0)';      tags = @('coercion', 'literal') }
                @{ formula = '=ROUND(A5,0)';         tags = @('error-input') }
                @{ formula = '=ROUND(1/3,15)';       tags = @('precision') }
                @{ formula = '=ROUND(-0.0,0)';       tags = @('boundary') }
                @{ formula = '=ROUND(B1,-300)';      tags = @('boundary', 'overflow'); note = 'the extreme is seeded via Value2; the literal is unparseable' }
            )
        }

        @{
            name = 'ROUNDUP'
            doc  = 'Away from zero, so it raises the magnitude of a negative operand.'
            cases = @(
                @{ formula = '=ROUNDUP(2.1,0)';      tags = @('basic') }
                @{ formula = '=ROUNDUP(-2.1,0)';     tags = @('negative'); note = 'away from zero, not toward +inf' }
                @{ formula = '=ROUNDUP(2.0,0)';      tags = @('boundary'); note = 'already exact' }
                @{ formula = '=ROUNDUP(0,0)';        tags = @('boundary') }
                @{ formula = '=ROUNDUP(-0.0001,0)';  tags = @('negative', 'boundary') }
                @{ formula = '=ROUNDUP(1.001,2)';    tags = @('basic') }
                @{ formula = '=ROUNDUP(1234.5678,-2)'; tags = @('basic') }
                @{ formula = '=ROUNDUP(0.1+0.2,1)';  tags = @('precision', 'compat-bug'); note = 'does the operand arrive as 0.3 or as 0.30000000000000004?' }
                @{ formula = '=ROUNDUP(A4,0)';       tags = @('coercion') }
                @{ formula = '=ROUNDUP(A5,0)';       tags = @('error-input') }
            )
        }

        @{
            name = 'ROUNDDOWN'
            doc  = 'Toward zero, so it is truncation rather than a floor.'
            cases = @(
                @{ formula = '=ROUNDDOWN(2.9,0)';      tags = @('basic') }
                @{ formula = '=ROUNDDOWN(-2.9,0)';     tags = @('negative'); note = 'toward zero, not toward -inf' }
                @{ formula = '=ROUNDDOWN(0,0)';        tags = @('boundary') }
                @{ formula = '=ROUNDDOWN(0.9999,0)';   tags = @('boundary') }
                @{ formula = '=ROUNDDOWN(-0.9999,0)';  tags = @('negative', 'boundary') }
                @{ formula = '=ROUNDDOWN(1.999,2)';    tags = @('basic') }
                @{ formula = '=ROUNDDOWN(1234.5678,-2)'; tags = @('basic') }
                @{ formula = '=ROUNDDOWN(0.1+0.2,1)';  tags = @('precision', 'compat-bug'); note = 'truncating 0.30000000000000004 at one digit would give 0.3 either way; truncating at 17 would not' }
                @{ formula = '=ROUNDDOWN(0.1+0.2,17)'; tags = @('precision', 'compat-bug') }
                @{ formula = '=ROUNDDOWN(A5,0)';       tags = @('error-input') }
            )
        }

        @{
            name = 'CEILING'
            doc  = 'Rounds away from zero to a multiple of significance. Zero significance is 0, not an error -- the opposite of FLOOR.'
            cases = @(
                @{ formula = '=CEILING(2.5,1)';    tags = @('basic') }
                @{ formula = '=CEILING(2.5,0.5)';  tags = @('basic') }
                @{ formula = '=CEILING(2.1,0.3)';  tags = @('precision'); note = '0.3 is not representable, so the multiple grid is approximate' }
                @{ formula = '=CEILING(-2.5,1)';   tags = @('negative'); note = 'sign of significance versus sign of number' }
                @{ formula = '=CEILING(-2.5,-1)';  tags = @('negative') }
                @{ formula = '=CEILING(2.5,-1)';   tags = @('error-input', 'boundary'); note = 'mixed signs' }
                @{ formula = '=CEILING(2.5,0)';    tags = @('boundary'); note = 'zero significance -- compare with FLOOR' }
                @{ formula = '=CEILING(0,0)';      tags = @('boundary') }
                @{ formula = '=CEILING(0,5)';      tags = @('boundary') }
                @{ formula = '=CEILING(2.5,A1)';   tags = @('blank') }
                @{ formula = '=CEILING(-2.5,0)';   tags = @('boundary', 'negative') }
                @{ formula = '=CEILING(A5,1)';     tags = @('error-input') }
            )
        }

        @{
            name = 'FLOOR'
            doc  = 'Rounds toward zero to a multiple of significance. A zero significance is #DIV/0!, where CEILING answers 0.'
            cases = @(
                @{ formula = '=FLOOR(2.5,1)';     tags = @('basic') }
                @{ formula = '=FLOOR(2.5,0.5)';   tags = @('basic') }
                @{ formula = '=FLOOR(2.9,0.3)';   tags = @('precision') }
                @{ formula = '=FLOOR(-2.5,1)';    tags = @('negative'); note = 'legacy Excel made this #NUM!' }
                @{ formula = '=FLOOR(-2.5,-1)';   tags = @('negative') }
                @{ formula = '=FLOOR(2.5,-1)';    tags = @('error-input', 'boundary') }
                @{ formula = '=FLOOR(2.5,0)';     tags = @('boundary', 'compat-bug'); note = 'asymmetric with CEILING(2.5,0)' }
                @{ formula = '=FLOOR(0,0)';       tags = @('boundary') }
                @{ formula = '=FLOOR(0,5)';       tags = @('boundary') }
                @{ formula = '=FLOOR(2.5,A1)';    tags = @('blank') }
                @{ formula = '=FLOOR(A5,1)';      tags = @('error-input') }
            )
        }
    )
}
