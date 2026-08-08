# The enumerated Excel bug catalogue of docs/32 and docs/12, captured rather than
# assumed. These entries are not functions, so they are named with a leading __
# to keep them distinguishable from the catalogue in functions.rs.
#
# Each entry answers a question the design documents currently answer from
# Microsoft's documentation, which docs/32 says outright is not trustworthy:
#
#   __compat_display_rounding  is the 15-digit rule display-only (D-041's
#                              compat_round_15) and what exactly is 15 digits of
#   __compat_cancellation      the compat_final_adjust threshold: how close to
#                              zero must a result be, relative to its operands,
#                              before Excel zeroes it. TD-13 records that the
#                              1e-15 relative threshold in the engine is
#                              documentation-derived and unvalidated. This is the
#                              vector set that validates or refutes it.
#   __compat_1900_leap         the phantom leap day from every direction
#   __compat_serial_boundary   the ends of the serial range
#   __compat_coercion          the implicit text/number/logical conversions
#   __compat_comparison        the cross-type comparison ordering, which is a
#                              total order over type tags, not a value order
#   __compat_precision_limits  where 15 significant digits stops being enough
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'compat-catalog'

    functions = @(
        @{
            name = '__compat_display_rounding'
            doc  = 'D-041 compat_round_15. The claim under test: Excel stores the full IEEE double and applies a 15-significant-digit rule only when rendering, so the stored value and the displayed value disagree. Each case records both the exact double and the text Excel splices into a string.'
            cases = @(
                @{ formula = '=0.1+0.2';                tags = @('display', 'precision'); note = 'the canonical case: stored 0.30000000000000004, shown 0.3' }
                @{ formula = '=1/3';                    tags = @('display', 'precision') }
                @{ formula = '=2/3';                    tags = @('display', 'precision'); note = 'does the last shown digit round or truncate' }
                @{ formula = '=1/7';                    tags = @('display', 'precision') }
                @{ formula = '=10/3';                   tags = @('display', 'precision'); note = 'an integer part shifts which digits survive' }
                @{ formula = '=1000000/3';              tags = @('display', 'precision') }
                @{ formula = '=1/3*3';                  tags = @('display', 'precision'); note = 'exactly 1, or 0.9999999999999999' }
                @{ formula = '=0.1*3';                  tags = @('display', 'precision') }
                @{ formula = '=1.1-1';                  tags = @('display', 'precision') }
                @{ formula = '=4.35*100';               tags = @('display', 'precision'); note = 'the classic currency-rounding failure' }
                @{ formula = '=1.005*100';              tags = @('display', 'precision') }
                @{ formula = '=0.1+0.7';                tags = @('display', 'precision') }
                @{ formula = '=(0.1+0.2)*10';           tags = @('display', 'precision'); note = 'does the error survive a multiply' }
                @{ formula = '=1E15+1';                 tags = @('display', 'boundary'); note = '16 digits: still exact in a double' }
                @{ formula = '=1E15+0.5';               tags = @('display', 'boundary') }
                @{ formula = '=1E16+1';                 tags = @('display', 'boundary'); note = '17 digits: no longer exact' }
                @{ formula = '=1E16+2';                 tags = @('display', 'boundary') }
                @{ formula = '=123456789012345678';     tags = @('display', 'boundary'); note = '18 digits entered as a literal' }
                @{ formula = '=1234567890123456789+1';  tags = @('display', 'boundary') }
                @{ formula = '=0.1+0.2=0.3';            tags = @('display', 'comparison'); note = 'does the display rule reach the = operator' }
                @{ formula = '=(0.1+0.2)-0.3=0';        tags = @('display', 'comparison') }
                @{ formula = '=TEXT(0.1+0.2,"0.00000000000000000000")'; tags = @('display'); note = 'TEXT with more digits than the rule allows' }
                @{ formula = '=(0.1+0.2)*1E17';         tags = @('display', 'precision'); note = 'scale the error up until it is visible' }
                @{ formula = '=1E-15+1';                tags = @('display', 'boundary') }
                @{ formula = '=1E-16+1';                tags = @('display', 'boundary'); note = 'below the double epsilon for 1.0' }
            )
        }

        @{
            name = '__compat_cancellation'
            doc  = 'D-041 compat_final_adjust and TD-13. Excel zeroes a result that is vanishingly small relative to its operands, which is why =0.1+0.2-0.3 is exactly 0 while the same arithmetic in IEEE is 5.55e-17. These cases walk the operand magnitude and the relative size of the residue to locate the actual threshold.'
            cases = @(
                @{ formula = '=0.1+0.2-0.3';        tags = @('cancellation'); note = 'the case the whole rule exists for' }
                @{ formula = '=0.3-0.1-0.2';        tags = @('cancellation'); note = 'same values, different order' }
                @{ formula = '=0.5-0.4-0.1';        tags = @('cancellation') }
                @{ formula = '=1.1-1-0.1';          tags = @('cancellation') }
                @{ formula = '=(0.1+0.2)-0.3';      tags = @('cancellation'); note = 'explicit parentheses: same result' }
                @{ formula = '=SUM(0.1,0.2)-0.3';   tags = @('cancellation'); note = 'does it survive crossing a function boundary' }
                @{ formula = '=SUM(0.1,0.2,-0.3)';  tags = @('cancellation'); note = 'entirely inside one SUM' }
                @{ formula = '=1E16+1-1E16';        tags = @('cancellation', 'boundary'); note = 'large operands, integer residue' }
                @{ formula = '=1E15+1-1E15';        tags = @('cancellation', 'boundary'); note = 'residue is exactly representable here' }
                @{ formula = '=1E16+2-1E16';        tags = @('cancellation', 'boundary') }
                @{ formula = '=1+1E-15-1';          tags = @('cancellation', 'boundary'); note = 'residue at the claimed 1e-15 threshold' }
                @{ formula = '=1+1E-14-1';          tags = @('cancellation', 'boundary'); note = 'one decade above it' }
                @{ formula = '=1+1E-16-1';          tags = @('cancellation', 'boundary'); note = 'one decade below it' }
                @{ formula = '=1+2E-16-1';          tags = @('cancellation', 'boundary') }
                @{ formula = '=1+1E-13-1';          tags = @('cancellation', 'boundary') }
                @{ formula = '=1000000+1E-9-1000000'; tags = @('cancellation', 'boundary'); note = 'relative residue 1e-15 at a larger scale' }
                @{ formula = '=1000000+1E-10-1000000'; tags = @('cancellation', 'boundary'); note = 'relative residue 1e-16' }
                @{ formula = '=1E-300+1E-300-1E-300'; tags = @('cancellation', 'boundary'); note = 'tiny operands of equal magnitude; a subnormal residue is unreachable from formula text' }
                @{ formula = '=0.1+0.2-0.3=0';      tags = @('cancellation', 'comparison') }
                @{ formula = '=ABS(0.1+0.2-0.3)>0'; tags = @('cancellation', 'comparison'); note = 'is the zero real or only displayed' }
                @{ formula = '=1/(0.1+0.2-0.3)';    tags = @('cancellation', 'boundary'); note = 'a real zero divides to #DIV/0!' }
                @{ formula = '=(0.1+0.2-0.3)*1E20'; tags = @('cancellation', 'boundary'); note = 'scaling a real zero stays zero' }
            )
        }

        @{
            name = '__compat_1900_leap'
            doc  = 'The phantom 29 February 1900, approached from serial arithmetic, DATE, the date-part functions, WEEKDAY, DATEVALUE and text formatting. Serial 60 is the fault line; every vector here says which side of it a given path lands on.'
            cases = @(
                @{ formula = '=DATE(1900,2,29)';               tags = @('1900-leap') }
                @{ formula = '=DAY(60)';                       tags = @('1900-leap') }
                @{ formula = '=MONTH(60)';                     tags = @('1900-leap') }
                @{ formula = '=YEAR(60)';                      tags = @('1900-leap') }
                @{ formula = '=TEXT(60,"yyyy-mm-dd")';         tags = @('1900-leap', 'display') }
                @{ formula = '=TEXT(59,"yyyy-mm-dd")';         tags = @('1900-leap', 'display') }
                @{ formula = '=TEXT(61,"yyyy-mm-dd")';         tags = @('1900-leap', 'display') }
                @{ formula = '=TEXT(1,"yyyy-mm-dd")';          tags = @('1900-leap', 'display'); note = 'the epoch' }
                @{ formula = '=TEXT(0,"yyyy-mm-dd")';          tags = @('1900-leap', 'display'); note = 'serial 0 formatted as a date' }
                @{ formula = '=DATE(1900,3,1)-DATE(1900,2,28)'; tags = @('1900-leap'); note = 'two days apart, not one' }
                @{ formula = '=DATE(1901,1,1)-DATE(1900,1,1)'; tags = @('1900-leap'); note = '366 if 1900 is a leap year here, 365 in reality' }
                @{ formula = '=DATEVALUE("1900-02-29")';       tags = @('1900-leap'); note = 'does the parser accept the phantom' }
                @{ formula = '=DATEVALUE("1900-02-28")';       tags = @('1900-leap') }
                @{ formula = '=DATEVALUE("1900-03-01")';       tags = @('1900-leap') }
                @{ formula = '=DATEVALUE("2024-02-29")';       tags = @('1900-leap'); note = 'a real leap day for contrast' }
                @{ formula = '=WEEKDAY(60)';                   tags = @('1900-leap') }
                @{ formula = '=DATE(1900,2,29)=60';            tags = @('1900-leap') }
                @{ formula = '=EOMONTH(DATE(1900,2,1),0)';     tags = @('1900-leap'); note = 'end of February 1900 per Excel' }
                @{ formula = '=DAY(EOMONTH(DATE(1900,2,1),0))'; tags = @('1900-leap'); note = '28 or 29' }
                @{ formula = '=DATE(1900,2,29)-DATE(1900,2,28)'; tags = @('1900-leap') }
                @{ formula = '=DATE(1900,2,29)+1';             tags = @('1900-leap') }
                @{ formula = '=TEXT(DATE(1900,2,29),"dddd")';  tags = @('1900-leap', 'display'); note = 'a weekday name for a day that never happened' }
            )
        }

        @{
            name = '__compat_serial_boundary'
            doc  = 'The ends of the serial range and the sub-day fraction. Where a serial stops being a date, whether it errors or clamps, and how much time resolution a double actually carries at serial 45000.'
            cases = @(
                @{ formula = '=YEAR(1)';           tags = @('serial-boundary') }
                @{ formula = '=YEAR(0)';           tags = @('serial-boundary'); note = 'serial 0 has no real date' }
                @{ formula = '=DAY(0)';            tags = @('serial-boundary') }
                @{ formula = '=MONTH(0)';          tags = @('serial-boundary') }
                @{ formula = '=TEXT(-1,"yyyy-mm-dd")'; tags = @('serial-boundary'); note = 'negative serials are not dates' }
                @{ formula = '=YEAR(-1)';          tags = @('serial-boundary') }
                @{ formula = '=YEAR(2958465)';     tags = @('serial-boundary'); note = '31 December 9999, the last date' }
                @{ formula = '=TEXT(2958465,"yyyy-mm-dd")'; tags = @('serial-boundary', 'display') }
                @{ formula = '=YEAR(2958466)';     tags = @('serial-boundary', 'overflow') }
                @{ formula = '=DATE(9999,12,31)=2958465'; tags = @('serial-boundary') }
                @{ formula = '=0.5';               tags = @('serial-boundary'); note = 'half a day' }
                @{ formula = '=TEXT(0.5,"hh:mm:ss")'; tags = @('serial-boundary', 'display'); note = 'noon' }
                @{ formula = '=TEXT(45292.75,"yyyy-mm-dd hh:mm:ss")'; tags = @('serial-boundary', 'display') }
                @{ formula = '=TEXT(1/86400,"hh:mm:ss")'; tags = @('serial-boundary', 'display'); note = 'one second as a fraction of a day' }
                @{ formula = '=45292+1/86400-45292';  tags = @('serial-boundary', 'precision'); note = 'is one second still resolvable at serial 45292' }
                @{ formula = '=45292+1/86400000-45292'; tags = @('serial-boundary', 'precision'); note = 'one millisecond' }
                @{ formula = '=TEXT(45292+1/86400,"hh:mm:ss")'; tags = @('serial-boundary', 'display') }
                @{ formula = '=INT(45292.99999999)';  tags = @('serial-boundary', 'precision') }
                @{ formula = '=TEXT(45292.99999999,"yyyy-mm-dd hh:mm:ss")'; tags = @('serial-boundary', 'display'); note = 'does it round up to the next day' }
            )
        }

        @{
            name = '__compat_coercion'
            doc  = 'The implicit conversions of the compat profile (docs/12). An operator context coerces text and logicals; a range context skips them. These vectors are the specification of Profile::Compat::to_number.'
            fixture = @(
                @{ ref = 'A1'; blank = $true }
                @{ ref = 'A2'; text = '1E2' }
                @{ ref = 'A3'; text = ' 5 ' }
                @{ ref = 'A4'; text = '$1,234.50' }
                @{ ref = 'A5'; text = '50%' }
                @{ ref = 'B1'; text = '1/2/2024' }
                @{ ref = 'B2'; text = '(5)' }
                @{ ref = 'B3'; text = '1 2' }
                @{ ref = 'B4'; text = 'TRUE' }
                @{ ref = 'B5'; formula = '=""' }
            )
            cases = @(
                @{ formula = '="1"+1';        tags = @('coercion'); note = 'text plus number' }
                @{ formula = '="1"&1';        tags = @('coercion'); note = 'concatenation never coerces' }
                @{ formula = '=1&1';          tags = @('coercion') }
                @{ formula = '="1"*"2"';      tags = @('coercion') }
                @{ formula = '=TRUE+1';       tags = @('coercion') }
                @{ formula = '=FALSE+1';      tags = @('coercion') }
                @{ formula = '=TRUE*2';       tags = @('coercion') }
                @{ formula = '=TRUE&""';      tags = @('coercion'); note = 'how a logical spells itself as text' }
                @{ formula = '=""+1';         tags = @('coercion', 'boundary'); note = 'an empty string is not zero' }
                @{ formula = '=" "+1';        tags = @('coercion', 'boundary'); note = 'a single space' }
                @{ formula = '=A1+1';         tags = @('coercion', 'blank'); note = 'a blank cell IS zero' }
                @{ formula = '=B5+1';         tags = @('coercion', 'boundary'); note = 'a formula-produced empty string' }
                @{ formula = '=A2*1';         tags = @('coercion', 'compat-bug'); note = 'the gene-symbol mangling: text 1E2 as a number' }
                @{ formula = '=A3*1';         tags = @('coercion'); note = 'surrounding spaces are tolerated' }
                @{ formula = '=A4*1';         tags = @('coercion'); note = 'currency symbol and thousands separator' }
                @{ formula = '=A5*1';         tags = @('coercion'); note = 'percent divides by 100' }
                @{ formula = '=B1+0';         tags = @('coercion'); note = 'a date string becomes its serial' }
                @{ formula = '=B2*1';         tags = @('coercion'); note = 'parenthesised negative' }
                @{ formula = '=B3*1';         tags = @('coercion', 'error-input'); note = 'a space between digits is not numeric' }
                @{ formula = '=B4+0';         tags = @('coercion'); note = 'the text TRUE in arithmetic' }
                @{ formula = '=-"1"';         tags = @('coercion'); note = 'unary minus on text' }
                @{ formula = '=+"1"';         tags = @('coercion') }
                @{ formula = '="1"="1.0"';    tags = @('coercion', 'comparison'); note = 'text comparison does not normalise numbers' }
                @{ formula = '=1="1"';        tags = @('coercion', 'comparison'); note = 'a number never equals text' }
                @{ formula = '="abc"+0';      tags = @('coercion', 'error-input') }
                @{ formula = '=A1&"x"';       tags = @('coercion', 'blank'); note = 'a blank concatenates as nothing' }
                @{ formula = '=N("1")';       tags = @('coercion'); note = 'the explicit numeric coercion function' }
                @{ formula = '=T(1)';         tags = @('coercion'); note = 'the explicit text coercion function' }
                @{ formula = '=T("1")';       tags = @('coercion') }
                @{ formula = '=--"1E2"';      tags = @('coercion'); note = 'the double-unary idiom' }
            )
        }

        @{
            name = '__compat_comparison'
            doc  = 'Cross-type comparison. Excel orders by type tag first -- number below text below logical -- so 1<"1" is TRUE for a reason that has nothing to do with the values. A blank compares equal to both 0 and "", which are not equal to each other.'
            fixture = @(
                @{ ref = 'A1'; blank = $true }
                @{ ref = 'A2'; value = 0 }
                @{ ref = 'A3'; value = '' }
                @{ ref = 'A4'; formula = '=""' }
                @{ ref = 'A5'; value = 'abc' }
            )
            cases = @(
                @{ formula = '="a"="A"';       tags = @('comparison'); note = '= is case-insensitive; EXACT is not' }
                @{ formula = '="a"<"B"';       tags = @('comparison') }
                @{ formula = '="a"<"b"';       tags = @('comparison') }
                @{ formula = '=1<"1"';         tags = @('comparison'); note = 'every number sorts below every text' }
                @{ formula = '=999999<"0"';    tags = @('comparison') }
                @{ formula = '="1"<TRUE';      tags = @('comparison'); note = 'every text sorts below every logical' }
                @{ formula = '=1<TRUE';        tags = @('comparison') }
                @{ formula = '=TRUE>1000000';  tags = @('comparison') }
                @{ formula = '=FALSE>"zzz"';   tags = @('comparison') }
                @{ formula = '=TRUE>FALSE';    tags = @('comparison') }
                @{ formula = '=A1=0';          tags = @('comparison', 'blank'); note = 'a blank equals zero' }
                @{ formula = '=A1=""';         tags = @('comparison', 'blank'); note = 'and equals an empty string' }
                @{ formula = '=0=""';          tags = @('comparison', 'boundary'); note = 'but those two do not equal each other' }
                @{ formula = '=A1=A3';         tags = @('comparison', 'blank') }
                @{ formula = '=A1=A4';         tags = @('comparison', 'blank'); note = 'blank against a formula-produced empty string' }
                @{ formula = '=A4=""';         tags = @('comparison', 'boundary') }
                @{ formula = '=A4=0';          tags = @('comparison', 'boundary') }
                @{ formula = '=A1<1';          tags = @('comparison', 'blank') }
                @{ formula = '=A1<"a"';        tags = @('comparison', 'blank'); note = 'which type tag does a blank take in a text comparison' }
                @{ formula = '=A1>-1';         tags = @('comparison', 'blank') }
                @{ formula = '=NA()=NA()';     tags = @('comparison', 'error-input'); note = 'errors do not compare, they propagate' }
                @{ formula = '=1<>"1"';        tags = @('comparison') }
                @{ formula = '="10"<"9"';      tags = @('comparison'); note = 'text compares lexicographically' }
                @{ formula = '=10<9';          tags = @('comparison') }
            )
        }

        @{
            name = '__compat_precision_limits'
            doc  = 'Where 15 significant digits stops being enough, and what Excel does at the edges of the double range: overflow to #NUM! rather than infinity, underflow to zero, and the largest exactly representable integer. The extremes arrive as fixture cells because Excel-s formula parser cannot express them at all -- see __compat_literal_parser.'
            fixture = @(
                @{ ref = 'A1'; value = 1.7976931348623157E+308 }
                @{ ref = 'A2'; value = 1E+308 }
                @{ ref = 'A3'; value = 5E-324 }
                @{ ref = 'A4'; value = 2.2250738585072014E-308 }
                @{ ref = 'A5'; value = 9007199254740992 }
            )
            cases = @(
                @{ formula = '=A1';                         tags = @('precision', 'boundary'); note = 'the largest finite double, seeded via Value2' }
                @{ formula = '=A1*2';                       tags = @('precision', 'overflow'); note = 'IEEE gives +Inf; Excel has no infinity' }
                @{ formula = '=A2*10';                      tags = @('precision', 'overflow') }
                @{ formula = '=A4';                         tags = @('precision', 'boundary'); note = 'the smallest normal double' }
                @{ formula = '=A4/1E+10';                   tags = @('precision', 'boundary'); note = 'divided down into the subnormal range' }
                @{ formula = '=A3';                         tags = @('precision', 'boundary'); note = 'the smallest positive subnormal -- does Excel keep it or flatten it' }
                @{ formula = '=A3*1E+10';                   tags = @('precision', 'boundary') }
                @{ formula = '=A3=0';                       tags = @('precision', 'boundary'); note = 'is the subnormal distinguishable from zero' }
                @{ formula = '=A4=0';                       tags = @('precision', 'boundary') }
                @{ formula = '=2^53';                       tags = @('precision', 'boundary'); note = '2^53, computed rather than written -- the literal would be 15-digit truncated' }
                @{ formula = '=A5';                         tags = @('precision', 'boundary'); note = '2^53 seeded exactly via Value2' }
                @{ formula = '=2^53+1';                     tags = @('precision', 'boundary'); note = '2^53+1 is not representable' }
                @{ formula = '=A5+1';                       tags = @('precision', 'boundary') }
                @{ formula = '=A5+2';                       tags = @('precision', 'boundary') }
                @{ formula = '=2^53=2^53+1';                tags = @('precision', 'boundary'); note = 'the collapse made visible' }
                @{ formula = '=999999999999999+1';          tags = @('precision', 'boundary') }
                @{ formula = '=1/0';                        tags = @('boundary'); note = 'no infinity in the value domain' }
                @{ formula = '=0/0';                        tags = @('boundary'); note = 'and no NaN' }
                @{ formula = '=LOG(0)';                     tags = @('boundary') }
                @{ formula = '=EXP(1000)';                  tags = @('overflow', 'boundary') }
                @{ formula = '=EXP(-1000)';                 tags = @('boundary'); note = 'underflow' }
                @{ formula = '=A2+A2-A2';                   tags = @('overflow', 'cancellation'); note = 'does an intermediate overflow survive being cancelled' }
                @{ formula = '=2^1024';                     tags = @('overflow', 'boundary') }
                @{ formula = '=2^1023';                     tags = @('boundary') }
                @{ formula = '=2^-1074';                    tags = @('boundary'); note = 'the smallest positive subnormal' }
                @{ formula = '=2^-1075';                    tags = @('boundary'); note = 'below it' }
            )
        }
    )
}
