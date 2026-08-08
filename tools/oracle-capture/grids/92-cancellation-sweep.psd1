# A targeted sweep to locate D-041's compat_final_adjust threshold, written
# after the first full capture refuted the rule as docs/43 states it.
#
# TD-13 records that `compat_final_adjust`'s 1e-15 *relative* threshold is
# documentation-derived and unvalidated. The first capture does not merely leave
# it unvalidated -- it contradicts it. Two cases with essentially the same
# relative residue disagree:
#
#   =1+1E-15-1              residue 1.1102e-15 / operand 1     = 1.11e-15  -> 0
#   =1000000+1E-9-1000000   residue 1.0477e-09 / operand 1e6   = 1.05e-15  -> kept
#
# No single relative bound produces both. So the rule depends on something else
# as well, and this sweep is built to find out what:
#
#   block 1  hold the operand at 1 and walk the residue by decades
#   block 2  hold the relative residue near 1e-15 and walk the operand magnitude
#            by decades -- if the rule were relative, every row would agree
#   block 3  walk the residue in ULPs of the operand rather than in decades,
#            which is what a threshold expressed in bits would track
#   block 4  where the adjustment applies. =0.1+0.2-0.3 gives 0, but
#            =1/(0.1+0.2-0.3) gives 2^54 and =ABS(0.1+0.2-0.3)>0 gives TRUE.
#            The residue is therefore still in the value and the zero is applied
#            to the formula's *final result* -- which is what the name says and
#            what an implementation must reproduce: this is not a property of the
#            subtraction operator.
#   block 5  which operations count as "final". If wrapping the subtraction
#            suppresses the adjustment, the rule is positional, and every one of
#            these wrappers is a place an engine could get it wrong.
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'compat-catalog'

    functions = @(
        @{
            name = '__compat_cancellation_sweep'
            doc  = 'Locating the compat_final_adjust boundary empirically, and establishing that the adjustment is applied to a formula result rather than to the subtraction operator.'
            blocks = @(
                @{
                    label = 'residue decades at operand magnitude 1'
                    cases = @(
                        @{ formula = '=1+1E-11-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+1E-12-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+1E-13-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+5E-14-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+2E-14-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+1E-14-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+5E-15-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+4E-15-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+3E-15-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+2E-15-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+1E-15-1'; tags = @('sweep', 'residue-decade'); note = 'zeroed in the first capture' }
                        @{ formula = '=1+5E-16-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+3E-16-1'; tags = @('sweep', 'residue-decade') }
                        @{ formula = '=1+2E-16-1'; tags = @('sweep', 'residue-decade') }
                    )
                }
                @{
                    label = 'operand magnitude decades at a fixed 1e-15 relative residue'
                    cases = @(
                        @{ formula = '=1+1E-15-1';               tags = @('sweep', 'operand-decade'); note = 'operand 1e0' }
                        @{ formula = '=10+1E-14-10';             tags = @('sweep', 'operand-decade'); note = 'operand 1e1' }
                        @{ formula = '=100+1E-13-100';           tags = @('sweep', 'operand-decade'); note = 'operand 1e2' }
                        @{ formula = '=1000+1E-12-1000';         tags = @('sweep', 'operand-decade'); note = 'operand 1e3' }
                        @{ formula = '=10000+1E-11-10000';       tags = @('sweep', 'operand-decade'); note = 'operand 1e4' }
                        @{ formula = '=100000+1E-10-100000';     tags = @('sweep', 'operand-decade'); note = 'operand 1e5' }
                        @{ formula = '=1000000+1E-9-1000000';    tags = @('sweep', 'operand-decade'); note = 'operand 1e6 -- kept in the first capture' }
                        @{ formula = '=10000000+1E-8-10000000';  tags = @('sweep', 'operand-decade'); note = 'operand 1e7' }
                        @{ formula = '=1E9+1E-6-1E9';            tags = @('sweep', 'operand-decade'); note = 'operand 1e9' }
                        @{ formula = '=1E12+0.001-1E12';         tags = @('sweep', 'operand-decade'); note = 'operand 1e12' }
                        @{ formula = '=1E15+1-1E15';             tags = @('sweep', 'operand-decade'); note = 'operand 1e15 -- kept in the first capture' }
                        @{ formula = '=0.1+1E-16-0.1';           tags = @('sweep', 'operand-decade'); note = 'operand 1e-1' }
                        @{ formula = '=0.001+1E-18-0.001';       tags = @('sweep', 'operand-decade'); note = 'operand 1e-3' }
                        @{ formula = '=1E-6+1E-21-1E-6';         tags = @('sweep', 'operand-decade'); note = 'operand 1e-6' }
                    )
                }
                @{
                    label = 'residue in ULPs of the operand'
                    fixture = @(
                        @{ ref = 'A1'; value = 1 }
                        @{ ref = 'A2'; value = 1000000 }
                        @{ ref = 'A3'; value = 1E+15 }
                    )
                    cases = @(
                        @{ formula = '=A1+2^-52-A1';   tags = @('sweep', 'ulp'); note = 'operand 1, residue 2 ULP' }
                        @{ formula = '=A1+2^-51-A1';   tags = @('sweep', 'ulp'); note = '4 ULP' }
                        @{ formula = '=A1+2^-50-A1';   tags = @('sweep', 'ulp'); note = '8 ULP' }
                        @{ formula = '=A1+2^-48-A1';   tags = @('sweep', 'ulp'); note = '32 ULP' }
                        @{ formula = '=A1+2^-46-A1';   tags = @('sweep', 'ulp'); note = '128 ULP' }
                        @{ formula = '=A1+2^-44-A1';   tags = @('sweep', 'ulp'); note = '512 ULP' }
                        @{ formula = '=A1+2^-40-A1';   tags = @('sweep', 'ulp'); note = '8192 ULP' }
                        @{ formula = '=A2+2^-32-A2';   tags = @('sweep', 'ulp'); note = 'operand 1e6' }
                        @{ formula = '=A2+2^-28-A2';   tags = @('sweep', 'ulp') }
                        @{ formula = '=A2+2^-24-A2';   tags = @('sweep', 'ulp') }
                        @{ formula = '=A3+1-A3';       tags = @('sweep', 'ulp'); note = 'operand 1e15' }
                        @{ formula = '=A3+2-A3';       tags = @('sweep', 'ulp') }
                        @{ formula = '=A3+8-A3';       tags = @('sweep', 'ulp') }
                    )
                }
                @{
                    label = 'is the zero in the value or only in the result'
                    cases = @(
                        @{ formula = '=0.1+0.2-0.3';               tags = @('final-adjust'); note = 'baseline: 0' }
                        @{ formula = '=(0.1+0.2-0.3)';             tags = @('final-adjust'); note = 'parenthesised: still final' }
                        @{ formula = '=+(0.1+0.2-0.3)';            tags = @('final-adjust'); note = 'unary plus after the subtraction' }
                        @{ formula = '=-(0.1+0.2-0.3)';            tags = @('final-adjust') }
                        @{ formula = '=1/(0.1+0.2-0.3)';           tags = @('final-adjust'); note = 'the residue reappears: 2^54' }
                        @{ formula = '=ABS(0.1+0.2-0.3)';          tags = @('final-adjust') }
                        @{ formula = '=ABS(0.1+0.2-0.3)>0';        tags = @('final-adjust'); note = 'TRUE in the first capture' }
                        @{ formula = '=(0.1+0.2-0.3)=0';           tags = @('final-adjust'); note = 'FALSE in the first capture' }
                        @{ formula = '=(0.1+0.2-0.3)*1';           tags = @('final-adjust'); note = 'multiplying by one is not a no-op here' }
                        @{ formula = '=(0.1+0.2-0.3)+0';           tags = @('final-adjust'); note = 'and the last op is an addition again' }
                        @{ formula = '=(0.1+0.2-0.3)-0';           tags = @('final-adjust') }
                        @{ formula = '=(0.1+0.2-0.3)*1E17';        tags = @('final-adjust') }
                        @{ formula = '=SIGN(0.1+0.2-0.3)';         tags = @('final-adjust') }
                        @{ formula = '=ISNUMBER(1/(0.1+0.2-0.3))'; tags = @('final-adjust') }
                        @{ formula = '=IF(0.1+0.2-0.3=0,"zero","nonzero")'; tags = @('final-adjust'); note = 'the form a user would actually write' }
                        @{ formula = '=IF(0.1+0.2-0.3,"truthy","falsy")';   tags = @('final-adjust') }
                        @{ formula = '=ROUND(0.1+0.2-0.3,20)';     tags = @('final-adjust') }
                        @{ formula = '=TEXT(0.1+0.2-0.3,"0.00000000000000000000")'; tags = @('final-adjust'); note = 'does TEXT see the residue' }
                    )
                }
                @{
                    label = 'the residue routed through a cell rather than an expression'
                    fixture = @(
                        @{ ref = 'A1'; formula = '=0.1+0.2-0.3' }
                        @{ ref = 'A2'; value = 0.1 }
                        @{ ref = 'A3'; value = 0.2 }
                        @{ ref = 'A4'; value = 0.3 }
                    )
                    cases = @(
                        @{ formula = '=A1';            tags = @('final-adjust', 'via-cell'); note = 'reading the adjusted cell' }
                        @{ formula = '=A1=0';          tags = @('final-adjust', 'via-cell'); note = 'is the stored cell value a true zero' }
                        @{ formula = '=1/A1';          tags = @('final-adjust', 'via-cell'); note = 'the decisive case: #DIV/0! means the cell really stores 0' }
                        @{ formula = '=A1*1E17';       tags = @('final-adjust', 'via-cell') }
                        @{ formula = '=ISNUMBER(1/A1)'; tags = @('final-adjust', 'via-cell') }
                        @{ formula = '=A2+A3-A4';      tags = @('final-adjust', 'via-cell'); note = 'the same arithmetic over cell references' }
                        @{ formula = '=1/(A2+A3-A4)';  tags = @('final-adjust', 'via-cell') }
                        @{ formula = '=SUM(A2:A3)-A4'; tags = @('final-adjust', 'via-cell') }
                        @{ formula = '=1/(SUM(A2:A3)-A4)'; tags = @('final-adjust', 'via-cell') }
                    )
                }
            )
        }
    )
}
