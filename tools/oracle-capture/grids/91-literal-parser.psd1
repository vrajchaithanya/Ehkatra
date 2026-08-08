# A rule none of the design documents mention: Excel's formula *parser* is
# itself lossy, before the evaluator ever runs.
#
# Discovered by this harness on the first full capture, because the harness
# records what Excel *stored* alongside what Excel *computed*. Three findings:
#
#   1. A numeric literal is truncated to 15 significant digits at parse time.
#      `=123456789012345678` is stored as `=123456789012345000`. So D-041's two
#      15-digit rules (compat_round_15 for display, compat_final_adjust for
#      cancellation) are incomplete: there is a third, at the front of the
#      pipeline, and it is destructive rather than cosmetic.
#   2. The exponent range accepted in formula text is narrower than a double's.
#      `=1E307` parses; `=1E308` is rejected outright even though it is a finite
#      double. `=1E-308` is accepted but silently stored as `=0`; `=1E-310` and
#      below are rejected. The top and bottom decades of the double range are
#      unreachable from formula text and can only be seeded as cell values.
#   3. Negative zero is normalised away: `=-0.0` is stored as `=0`.
#
# A rejected formula is a result here, not a gap. The harness records it as
# observed_status = 'rejected-by-excel' with the parser's own error, which is a
# conformance fact our parser has to match.
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'compat-catalog'

    functions = @(
        @{
            name = '__compat_literal_parser'
            doc  = 'What Excel does to a numeric literal before evaluating it. Read the stored_formula field of each case, not only the observed value.'
            cases = @(
                @{ formula = '=999999999999999';       tags = @('literal-parser', 'precision'); note = '15 nines survive intact' }
                @{ formula = '=9999999999999999';      tags = @('literal-parser', 'precision'); note = '16 nines do not' }
                @{ formula = '=123456789012345678';    tags = @('literal-parser', 'precision'); note = '18 digits in, 15 significant digits stored' }
                @{ formula = '=1234567890123456789';   tags = @('literal-parser', 'precision') }
                @{ formula = '=1.234567890123456789';  tags = @('literal-parser', 'precision'); note = 'truncation on the fractional side too' }
                @{ formula = '=0.1234567890123456789'; tags = @('literal-parser', 'precision') }
                @{ formula = '=1E307';                 tags = @('literal-parser', 'boundary'); note = 'the largest exponent the parser accepts' }
                @{ formula = '=1E308';                 tags = @('literal-parser', 'boundary'); note = 'rejected, though it is a finite double' }
                @{ formula = '=1E+308';                tags = @('literal-parser', 'boundary'); note = 'rejected with an explicit sign as well' }
                @{ formula = '=1.5E308';               tags = @('literal-parser', 'boundary'); note = 'rejected' }
                @{ formula = '=-1E308';                tags = @('literal-parser', 'boundary'); note = 'rejected on the negative side too' }
                @{ formula = '=1E309';                 tags = @('literal-parser', 'boundary') }
                @{ formula = '=1E-300';                tags = @('literal-parser', 'boundary') }
                @{ formula = '=1E-307';                tags = @('literal-parser', 'boundary') }
                @{ formula = '=1E-308';                tags = @('literal-parser', 'boundary'); note = 'accepted, then silently stored as 0 -- a normal double erased at parse time' }
                @{ formula = '=1E-309';                tags = @('literal-parser', 'boundary') }
                @{ formula = '=1E-310';                tags = @('literal-parser', 'boundary'); note = 'rejected' }
                @{ formula = '=1E-320';                tags = @('literal-parser', 'boundary'); note = 'rejected' }
                @{ formula = '=-0.0';                  tags = @('literal-parser', 'boundary'); note = 'negative zero is normalised away in the stored text' }
                @{ formula = '=-0';                    tags = @('literal-parser', 'boundary') }
                @{ formula = '=0*-1';                  tags = @('literal-parser', 'boundary'); note = 'can negative zero be reached by arithmetic instead' }
                @{ formula = '=1/(0*-1)';              tags = @('literal-parser', 'boundary'); note = 'if it could, IEEE would give -inf here rather than an error' }
                @{ formula = '=SIGN(0*-1)';            tags = @('literal-parser', 'boundary') }
                @{ formula = '=2.0';                   tags = @('literal-parser'); note = 'a trailing zero is dropped from the stored text' }
                @{ formula = '=1E15';                  tags = @('literal-parser'); note = 'exponent notation expands to plain digits when stored' }
                @{ formula = '=1E20';                  tags = @('literal-parser'); note = 'where does the expansion stop' }
                @{ formula = '=1E25';                  tags = @('literal-parser') }
                @{ formula = '=0.000000000000001';     tags = @('literal-parser'); note = 'and small values contract back into exponent form' }
                @{ formula = '=0.00001';               tags = @('literal-parser') }
            )
        }
    )
}
