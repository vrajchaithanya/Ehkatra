# Does compat_final_adjust's threshold track ULPs or a decimal ratio?
#
# The decade sweep in 92-cancellation-sweep.psd1 found the boundary between 7 and
# 8 ULP of the larger operand, monotone over eight decades. But a decade sweep
# cannot separate "8 ULP" from "roughly 1.7e-15 relative", because at operands
# that are powers of ten the two nearly coincide.
#
# A binade separates them cleanly. Inside [2^e, 2^(e+1)) the ULP is constant, so
# a fixed ULP count has a relative residue that halves from the bottom of the
# binade to the top. Probing both ends makes the two candidate rules predict
# *opposite* things, because the values cross over:
#
#     a = 1.0  k = 7 ULP    relative residue 1.5543e-15
#     a = 1.9  k = 8 ULP    relative residue 9.3492e-16   <- SMALLER
#
# A ULP rule says: zero the first, keep the second.
# Any single relative threshold says the opposite, or treats them the same.
# There is no relative threshold that reproduces zero-then-keep across that pair.
#
# Every operand below is seeded through Value2, and every residue is built as
# k * 2^n with n the binade's ULP exponent, so the residue is exactly k ULP by
# construction rather than approximately so. Both operands of the final
# subtraction stay inside one binade, which is what makes the arithmetic exact.
#
# Blocks:
#   1  full k sweep at both ends of the binade [1,2)
#   2  the paired discriminator at six binades from 2^-10 to 2^49
#   3  a residue that crosses a binade boundary, where ULP changes mid-operation
#   4  direct subtraction of two seeded doubles -- no addition involved at all
#   5  SUM over many addends, which the earlier capture showed is also adjusted
#   6  negative and mixed-sign operands
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'compat-catalog'

    # Operands, one pair per binade: bottom, then near the top. ULP exponent in
    # the comment is what the probe formulas multiply.
    fixture = @(
        @{ ref = 'A1'; value = 1.0 }                 # binade   0, ulp 2^-52
        @{ ref = 'A2'; value = 1.9 }                 # binade   0, near top
        @{ ref = 'A3'; value = 1.5 }                 # binade   0, mid
        @{ ref = 'A4'; value = 1.99999999999999 }     # binade   0, very near top
        @{ ref = 'B1'; value = 2.0 }                 # binade   1, ulp 2^-51
        @{ ref = 'B2'; value = 3.9 }                 # binade   1, near top
        @{ ref = 'C1'; value = 1024.0 }              # binade  10, ulp 2^-42
        @{ ref = 'C2'; value = 2047.9 }              # binade  10, near top
        @{ ref = 'D1'; value = 1048576.0 }           # binade  20, ulp 2^-32
        @{ ref = 'D2'; value = 2076180.48 }          # binade  20, near top
        @{ ref = 'E1'; value = 562949953421312.0 }   # binade  49, ulp 2^-3
        @{ ref = 'E2'; value = 1110000000000000.0 }  # binade  49, near top
        @{ ref = 'F1'; value = 0.0009765625 }        # binade -10, ulp 2^-62
        @{ ref = 'F2'; value = 0.0019 }              # binade -10, near top
        # The largest double below 2, for the binade-crossing block.
        @{ ref = 'G1'; formula = '=2-2^-52' }
        @{ ref = 'G2'; formula = '=4-2^-51' }
        # Pairs a fixed number of ULP apart, for the direct-subtraction block.
        @{ ref = 'H1'; formula = '=1+7*2^-52' }
        @{ ref = 'H2'; formula = '=1+8*2^-52' }
        @{ ref = 'H3'; formula = '=1.9+7*2^-52' }
        @{ ref = 'H4'; formula = '=1.9+8*2^-52' }
    )

    functions = @(
        @{
            name = '__compat_cancellation_binade'
            doc  = 'Separating a ULP-based threshold from a decimal-relative one by probing both ends of a binade, where the two rules predict opposite outcomes.'
            blocks = @(
                @{
                    label = 'full ULP sweep at the bottom of binade [1,2), operand 1.0'
                    cases = @(
                        @{ formula = '=A1+1*2^-52-A1';  tags = @('binade', 'sweep-bottom'); note = '1 ULP' }
                        @{ formula = '=A1+2*2^-52-A1';  tags = @('binade', 'sweep-bottom'); note = '2 ULP' }
                        @{ formula = '=A1+3*2^-52-A1';  tags = @('binade', 'sweep-bottom'); note = '3 ULP' }
                        @{ formula = '=A1+4*2^-52-A1';  tags = @('binade', 'sweep-bottom'); note = '4 ULP' }
                        @{ formula = '=A1+5*2^-52-A1';  tags = @('binade', 'sweep-bottom'); note = '5 ULP' }
                        @{ formula = '=A1+6*2^-52-A1';  tags = @('binade', 'sweep-bottom'); note = '6 ULP' }
                        @{ formula = '=A1+7*2^-52-A1';  tags = @('binade', 'sweep-bottom'); note = '7 ULP -- last zeroed in the decade sweep' }
                        @{ formula = '=A1+8*2^-52-A1';  tags = @('binade', 'sweep-bottom'); note = '8 ULP -- first kept in the decade sweep' }
                        @{ formula = '=A1+9*2^-52-A1';  tags = @('binade', 'sweep-bottom'); note = '9 ULP' }
                        @{ formula = '=A1+10*2^-52-A1'; tags = @('binade', 'sweep-bottom'); note = '10 ULP' }
                        @{ formula = '=A1+12*2^-52-A1'; tags = @('binade', 'sweep-bottom'); note = '12 ULP' }
                        @{ formula = '=A1+16*2^-52-A1'; tags = @('binade', 'sweep-bottom'); note = '16 ULP' }
                    )
                }
                @{
                    label = 'full ULP sweep near the top of binade [1,2), operand 1.9'
                    cases = @(
                        @{ formula = '=A2+1*2^-52-A2';  tags = @('binade', 'sweep-top'); note = '1 ULP' }
                        @{ formula = '=A2+2*2^-52-A2';  tags = @('binade', 'sweep-top'); note = '2 ULP' }
                        @{ formula = '=A2+3*2^-52-A2';  tags = @('binade', 'sweep-top'); note = '3 ULP' }
                        @{ formula = '=A2+4*2^-52-A2';  tags = @('binade', 'sweep-top'); note = '4 ULP' }
                        @{ formula = '=A2+5*2^-52-A2';  tags = @('binade', 'sweep-top'); note = '5 ULP' }
                        @{ formula = '=A2+6*2^-52-A2';  tags = @('binade', 'sweep-top'); note = '6 ULP' }
                        @{ formula = '=A2+7*2^-52-A2';  tags = @('binade', 'sweep-top'); note = '7 ULP, relative 8.18e-16' }
                        @{ formula = '=A2+8*2^-52-A2';  tags = @('binade', 'sweep-top', 'discriminator'); note = '8 ULP, relative 9.35e-16 -- SMALLER than 7 ULP at operand 1.0. Kept here means the rule is ULP-based, full stop.' }
                        @{ formula = '=A2+9*2^-52-A2';  tags = @('binade', 'sweep-top'); note = '9 ULP' }
                        @{ formula = '=A2+10*2^-52-A2'; tags = @('binade', 'sweep-top'); note = '10 ULP' }
                        @{ formula = '=A2+12*2^-52-A2'; tags = @('binade', 'sweep-top'); note = '12 ULP' }
                        @{ formula = '=A2+16*2^-52-A2'; tags = @('binade', 'sweep-top'); note = '16 ULP' }
                        @{ formula = '=A3+7*2^-52-A3';  tags = @('binade', 'sweep-mid'); note = 'mid-binade, 7 ULP' }
                        @{ formula = '=A3+8*2^-52-A3';  tags = @('binade', 'sweep-mid'); note = 'mid-binade, 8 ULP' }
                        @{ formula = '=A4+7*2^-52-A4';  tags = @('binade', 'sweep-top'); note = 'operand within 1 ULP of 2, 7 ULP' }
                        @{ formula = '=A4+8*2^-52-A4';  tags = @('binade', 'sweep-top'); note = 'operand within 1 ULP of 2, 8 ULP' }
                    )
                }
                @{
                    label = 'the paired discriminator across six binades'
                    cases = @(
                        @{ formula = '=B1+7*2^-51-B1'; tags = @('binade', 'paired'); note = 'binade 1 bottom, 7 ULP' }
                        @{ formula = '=B1+8*2^-51-B1'; tags = @('binade', 'paired'); note = 'binade 1 bottom, 8 ULP' }
                        @{ formula = '=B2+7*2^-51-B2'; tags = @('binade', 'paired'); note = 'binade 1 top, 7 ULP, relative 7.97e-16' }
                        @{ formula = '=B2+8*2^-51-B2'; tags = @('binade', 'paired', 'discriminator'); note = 'binade 1 top, 8 ULP, relative 9.11e-16' }
                        @{ formula = '=C1+7*2^-42-C1'; tags = @('binade', 'paired'); note = 'binade 10 bottom, 7 ULP' }
                        @{ formula = '=C1+8*2^-42-C1'; tags = @('binade', 'paired'); note = 'binade 10 bottom, 8 ULP' }
                        @{ formula = '=C2+7*2^-42-C2'; tags = @('binade', 'paired'); note = 'binade 10 top, 7 ULP' }
                        @{ formula = '=C2+8*2^-42-C2'; tags = @('binade', 'paired', 'discriminator'); note = 'binade 10 top, 8 ULP, relative 8.88e-16' }
                        @{ formula = '=D1+7*2^-32-D1'; tags = @('binade', 'paired'); note = 'binade 20 bottom, 7 ULP' }
                        @{ formula = '=D1+8*2^-32-D1'; tags = @('binade', 'paired'); note = 'binade 20 bottom, 8 ULP' }
                        @{ formula = '=D2+7*2^-32-D2'; tags = @('binade', 'paired'); note = 'binade 20 top, 7 ULP' }
                        @{ formula = '=D2+8*2^-32-D2'; tags = @('binade', 'paired', 'discriminator'); note = 'binade 20 top, 8 ULP' }
                        @{ formula = '=E1+7*2^-3-E1';  tags = @('binade', 'paired'); note = 'binade 49 bottom, 7 ULP -- residue is 0.875, a human-sized number' }
                        @{ formula = '=E1+8*2^-3-E1';  tags = @('binade', 'paired'); note = 'binade 49 bottom, 8 ULP -- residue is exactly 1' }
                        @{ formula = '=E2+7*2^-3-E2';  tags = @('binade', 'paired'); note = 'binade 49 top, 7 ULP' }
                        @{ formula = '=E2+8*2^-3-E2';  tags = @('binade', 'paired', 'discriminator'); note = 'binade 49 top, 8 ULP, relative 9.01e-16' }
                        @{ formula = '=F1+7*2^-62-F1'; tags = @('binade', 'paired'); note = 'binade -10 bottom, 7 ULP' }
                        @{ formula = '=F1+8*2^-62-F1'; tags = @('binade', 'paired'); note = 'binade -10 bottom, 8 ULP' }
                        @{ formula = '=F2+7*2^-62-F2'; tags = @('binade', 'paired'); note = 'binade -10 top, 7 ULP' }
                        @{ formula = '=F2+8*2^-62-F2'; tags = @('binade', 'paired', 'discriminator'); note = 'binade -10 top, 8 ULP, relative 9.13e-16' }
                    )
                }
                @{
                    label = 'residue crossing a binade boundary'
                    cases = @(
                        @{ formula = '=G1+1*2^-52-G1';  tags = @('binade', 'crossing'); note = 'largest double below 2, plus 1 ULP: the sum lands exactly on 2, where ULP doubles' }
                        @{ formula = '=G1+2*2^-52-G1';  tags = @('binade', 'crossing'); note = 'the sum is past 2 and no longer a clean multiple of the operand ULP' }
                        @{ formula = '=G1+8*2^-52-G1';  tags = @('binade', 'crossing'); note = '8 ULP of the operand, but only 4 ULP of the sum' }
                        @{ formula = '=G1+16*2^-52-G1'; tags = @('binade', 'crossing'); note = '8 ULP of the sum' }
                        @{ formula = '=G1+32*2^-52-G1'; tags = @('binade', 'crossing'); note = '16 ULP of the sum' }
                        @{ formula = '=G2+8*2^-51-G2';  tags = @('binade', 'crossing'); note = 'same shape one binade up' }
                        @{ formula = '=G2+16*2^-51-G2'; tags = @('binade', 'crossing') }
                        @{ formula = '=2-G1';           tags = @('binade', 'crossing'); note = 'the 1-ULP gap subtracted directly: is a genuine 1-ULP difference zeroed' }
                        @{ formula = '=4-G2';           tags = @('binade', 'crossing') }
                    )
                }
                @{
                    label = 'direct subtraction of two seeded doubles, no addition in the formula'
                    cases = @(
                        @{ formula = '=H1-A1';   tags = @('binade', 'direct-sub'); note = 'operand 1.0, 7 ULP apart' }
                        @{ formula = '=H2-A1';   tags = @('binade', 'direct-sub'); note = 'operand 1.0, 8 ULP apart' }
                        @{ formula = '=A1-H1';   tags = @('binade', 'direct-sub'); note = 'reversed, so the result is negative' }
                        @{ formula = '=A1-H2';   tags = @('binade', 'direct-sub') }
                        @{ formula = '=H3-A2';   tags = @('binade', 'direct-sub'); note = 'operand 1.9, 7 ULP apart' }
                        @{ formula = '=H4-A2';   tags = @('binade', 'direct-sub', 'discriminator'); note = 'operand 1.9, 8 ULP apart -- the discriminator without any addition' }
                        @{ formula = '=H1+-A1';  tags = @('binade', 'direct-sub'); note = 'addition of a negated operand rather than subtraction' }
                        @{ formula = '=H2+-A1';  tags = @('binade', 'direct-sub') }
                        @{ formula = '=H2-A1-0'; tags = @('binade', 'direct-sub'); note = 'a trailing -0 keeps the final operator a subtraction but changes its operands' }
                        # The parenthesis probe must use a residue that WOULD be
                        # zeroed, or it proves nothing: H2-A1 is 8 ULP and is kept
                        # either way. H1-A1 is 7 ULP, so the bare form is zeroed
                        # and any difference is attributable to the parentheses.
                        @{ formula = '=(H1-A1)';   tags = @('binade', 'direct-sub', 'positional'); note = '7 ULP parenthesised: bare =H1-A1 is zeroed, so a non-zero here isolates the positional rule' }
                        @{ formula = '=((H1-A1))'; tags = @('binade', 'direct-sub', 'positional'); note = 'doubly parenthesised' }
                        @{ formula = '=-(H1-A1)';  tags = @('binade', 'direct-sub', 'positional'); note = 'unary minus outside' }
                        @{ formula = '=(H1)-A1';   tags = @('binade', 'direct-sub', 'positional'); note = 'parentheses around an operand, not around the subtraction' }
                        @{ formula = '=H1-(A1)';   tags = @('binade', 'direct-sub', 'positional') }
                        @{ formula = '=1/(H1-A1)'; tags = @('binade', 'direct-sub', 'positional'); note = 'the residue should reappear, as it does for 1/(0.1+0.2-0.3)' }
                    )
                }
                @{
                    label = 'SUM and other aggregates, which the earlier capture showed are also adjusted'
                    cases = @(
                        @{ formula = '=SUM(A1,7*2^-52,-A1)';        tags = @('binade', 'aggregate'); note = '7 ULP inside one SUM' }
                        @{ formula = '=SUM(A1,8*2^-52,-A1)';        tags = @('binade', 'aggregate'); note = '8 ULP inside one SUM -- same threshold?' }
                        @{ formula = '=SUM(A2,7*2^-52,-A2)';        tags = @('binade', 'aggregate'); note = 'top-of-binade, 7 ULP' }
                        @{ formula = '=SUM(A2,8*2^-52,-A2)';        tags = @('binade', 'aggregate', 'discriminator'); note = 'top-of-binade, 8 ULP' }
                        @{ formula = '=SUM(A1,-A1,8*2^-52)';        tags = @('binade', 'aggregate'); note = 'cancellation first, then the residue added' }
                        @{ formula = '=SUM(H2,-A1)';                tags = @('binade', 'aggregate'); note = 'two-term SUM, 8 ULP apart' }
                        @{ formula = '=SUM(H1,-A1)';                tags = @('binade', 'aggregate'); note = 'two-term SUM, 7 ULP apart' }
                        @{ formula = '=SUM(A1,8*2^-52)-A1';         tags = @('binade', 'aggregate'); note = 'the subtraction outside the SUM' }
                        @{ formula = '=AVERAGE(A1,8*2^-52,-A1)*3';  tags = @('binade', 'aggregate'); note = 'the final operator is a multiply' }
                        @{ formula = '=SUM(A1,8*2^-52,-A1)*1';      tags = @('binade', 'aggregate'); note = 'multiplying by one after an adjusted SUM' }
                        @{ formula = '=1/SUM(A1,8*2^-52,-A1)';      tags = @('binade', 'aggregate'); note = 'is the SUM zero real or positional' }
                        @{ formula = '=1/SUM(A1,7*2^-52,-A1)';      tags = @('binade', 'aggregate'); note = '#DIV/0! here would mean SUM stores a true zero' }
                        @{ formula = '=SUMPRODUCT(A1,1)+8*2^-52-A1'; tags = @('binade', 'aggregate') }
                    )
                }
                @{
                    label = 'SUM adjusts intrinsically, the operator adjusts positionally'
                    # The first sweep found =1/(0.1+0.2-0.3) keeps the residue --
                    # the operator's adjustment is suppressed when the subtraction
                    # is not the final operation. But =1/SUM(A1,7*2^-52,-A1) gave
                    # #DIV/0!, meaning SUM's zero survived being nested. If that
                    # holds, these are two different mechanisms and an engine needs
                    # both: a positional rule for + and -, and an unconditional one
                    # inside the aggregates.
                    cases = @(
                        @{ formula = '=1/SUM(A1,7*2^-52,-A1)';       tags = @('mechanism'); note = '#DIV/0! means SUM zeroed unconditionally, unlike the operator' }
                        @{ formula = '=1/(A1+7*2^-52-A1)';           tags = @('mechanism'); note = 'the operator form of the same arithmetic, nested identically' }
                        @{ formula = '=SUM(A1,7*2^-52,-A1)=0';       tags = @('mechanism') }
                        @{ formula = '=(A1+7*2^-52-A1)=0';           tags = @('mechanism') }
                        @{ formula = '=SUM(A1,7*2^-52,-A1)*1E17';    tags = @('mechanism') }
                        @{ formula = '=(A1+7*2^-52-A1)*1E17';        tags = @('mechanism') }
                        @{ formula = '=ISNUMBER(1/SUM(A1,7*2^-52,-A1))'; tags = @('mechanism') }
                        @{ formula = '=SUM(SUM(A1,7*2^-52,-A1),0)';  tags = @('mechanism'); note = 'a SUM of an adjusted SUM' }
                        @{ formula = '=AVERAGE(A1,7*2^-52,-A1)';     tags = @('mechanism'); note = 'does AVERAGE adjust too' }
                        @{ formula = '=1/AVERAGE(A1,7*2^-52,-A1)';   tags = @('mechanism') }
                        @{ formula = '=1/SUMPRODUCT(A1,7*2^-52,-A1)'; tags = @('mechanism'); note = 'SUMPRODUCT multiplies, so this should not cancel at all' }
                        @{ formula = '=1/SUBTOTAL(9,A1:A1)';         tags = @('mechanism'); note = 'control: SUBTOTAL over a single cell' }
                        @{ formula = '=1/MAX(A1+7*2^-52-A1,0)';      tags = @('mechanism'); note = 'the subtraction nested in MAX rather than in a division' }
                        @{ formula = '=1/ABS(A1+7*2^-52-A1)';        tags = @('mechanism') }
                        @{ formula = '=1/(0+(A1+7*2^-52-A1))';       tags = @('mechanism'); note = 'forcing the subtraction to be a sub-expression another way' }
                    )
                }
                @{
                    label = 'negative and mixed-sign operands'
                    cases = @(
                        @{ formula = '=-A1-7*2^-52+A1'; tags = @('binade', 'negative'); note = 'both operands negative, 7 ULP' }
                        @{ formula = '=-A1-8*2^-52+A1'; tags = @('binade', 'negative'); note = 'both operands negative, 8 ULP' }
                        @{ formula = '=-A2-7*2^-52+A2'; tags = @('binade', 'negative'); note = 'top-of-binade, negative, 7 ULP' }
                        @{ formula = '=-A2-8*2^-52+A2'; tags = @('binade', 'negative', 'discriminator'); note = 'top-of-binade, negative, 8 ULP' }
                        @{ formula = '=A1-A1';          tags = @('binade', 'negative'); note = 'an exact zero needs no adjustment' }
                        @{ formula = '=A1+-A1';         tags = @('binade', 'negative') }
                        @{ formula = '=7*2^-52+A1-A1';  tags = @('binade', 'negative'); note = 'same values, residue added first -- the final subtraction now cancels the operands' }
                        @{ formula = '=8*2^-52+A1-A1';  tags = @('binade', 'negative') }
                        @{ formula = '=A1-A1+7*2^-52';  tags = @('binade', 'negative'); note = 'cancellation first, so the final addition has no cancellation to adjust' }
                        @{ formula = '=A1-A1+8*2^-52';  tags = @('binade', 'negative') }
                    )
                }
            )
        }
    )
}
