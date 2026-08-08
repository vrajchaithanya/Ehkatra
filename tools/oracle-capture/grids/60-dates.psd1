# Dates. The 1900 date system, its phantom leap day, and the serial boundaries.
#
# Serial 60 is 29 February 1900 -- a day that never existed, inherited from Lotus
# 1-2-3 so that 1900 could be treated as a leap year. Every serial after it is
# therefore one larger than a correct day count from the epoch. functions.rs
# reproduces this under Profile::Compat and refuses it under Profile::Strict, so
# these vectors are the acceptance test for both halves of that split.
#
# DATE also silently rewrites out-of-range arguments: a year below 1900 has 1900
# added to it, and months and days roll over rather than erroring. Neither is
# obvious and both change answers on imported data.
#
# TODAY and NOW are volatile, so no fixed vector can exist for them. What is
# captured instead is their structural relationship -- integrality, ordering,
# self-consistency -- which is testable without a clock.
@{
    schema = 'ehkatra.oracle.grid/1'
    group  = 'dates'

    fixture = @(
        @{ ref = 'A1'; blank = $true }
        @{ ref = 'A2'; value = 59 }
        @{ ref = 'A3'; value = 60 }
        @{ ref = 'A4'; value = 61 }
        @{ ref = 'A5'; value = 45292 }
        @{ ref = 'B1'; text  = '2024-03-15' }
        @{ ref = 'B2'; text  = '15/03/2024' }
        @{ ref = 'B3'; formula = '=NA()' }
        @{ ref = 'B4'; value = 2958465 }
        @{ ref = 'B5'; value = 0 }
        @{ ref = 'C1'; value = 45292.75 }
        @{ ref = 'C2'; value = -1 }
        @{ ref = 'C3'; value = 1 }
    )

    functions = @(
        @{
            name = 'DATE'
            doc  = 'Rolls over out-of-range months and days, and adds 1900 to any year below 1900. Serial 60 is reachable and is 29 February 1900.'
            cases = @(
                @{ formula = '=DATE(2024,1,1)';        tags = @('basic') }
                @{ formula = '=DATE(1900,1,1)';        tags = @('boundary'); note = 'serial 1, the epoch' }
                @{ formula = '=DATE(1900,2,28)';       tags = @('compat-bug', 'boundary'); note = 'serial 59' }
                @{ formula = '=DATE(1900,2,29)';       tags = @('compat-bug', 'boundary'); note = 'a date that never existed: serial 60' }
                @{ formula = '=DATE(1900,3,1)';        tags = @('compat-bug', 'boundary'); note = 'serial 61, one past the phantom day' }
                @{ formula = '=DATE(1900,1,0)';        tags = @('boundary'); note = 'serial 0' }
                @{ formula = '=DATE(1900,1,-1)';       tags = @('boundary'); note = 'negative serial' }
                @{ formula = '=DATE(1899,12,31)';      tags = @('compat-bug'); note = 'year below 1900 gets 1900 added: 3799, not 1899' }
                @{ formula = '=DATE(0,1,1)';           tags = @('compat-bug', 'boundary'); note = 'year 0 becomes 1900' }
                @{ formula = '=DATE(100,1,1)';         tags = @('compat-bug'); note = 'year 100 becomes 2000' }
                @{ formula = '=DATE(1899,1,1)';        tags = @('compat-bug'); note = 'the last year that gets shifted' }
                @{ formula = '=DATE(1900,0,1)';        tags = @('boundary'); note = 'month 0 rolls back a year' }
                @{ formula = '=DATE(2024,13,1)';       tags = @('rollover'); note = 'month 13 is January 2025' }
                @{ formula = '=DATE(2024,25,1)';       tags = @('rollover') }
                @{ formula = '=DATE(2024,0,1)';        tags = @('rollover') }
                @{ formula = '=DATE(2024,-1,1)';       tags = @('rollover') }
                @{ formula = '=DATE(2024,1,0)';        tags = @('rollover'); note = 'day 0 is the last day of the previous month' }
                @{ formula = '=DATE(2024,1,32)';       tags = @('rollover') }
                @{ formula = '=DATE(2024,2,30)';       tags = @('rollover'); note = '2024 is a real leap year' }
                @{ formula = '=DATE(2023,2,29)';       tags = @('rollover'); note = '2023 is not' }
                @{ formula = '=DATE(2024,1,-5)';       tags = @('rollover') }
                @{ formula = '=DATE(9999,12,31)';      tags = @('boundary'); note = 'the last representable date' }
                @{ formula = '=DATE(10000,1,1)';       tags = @('boundary', 'overflow') }
                @{ formula = '=DATE(-1,1,1)';          tags = @('error-input', 'boundary') }
                @{ formula = '=DATE(2024,1,1.9)';      tags = @('coercion'); note = 'fractional day' }
                @{ formula = '=DATE("2024","1","1")';  tags = @('coercion') }
                @{ formula = '=DATE(A1,A1,A1)';        tags = @('blank'); note = 'all-blank arguments' }
                @{ formula = '=DATE(B3,1,1)';          tags = @('error-input') }
                @{ formula = '=DATE(2024,1,1)-DATE(2023,1,1)'; tags = @('basic'); note = 'day count across a year' }
                @{ formula = '=DATE(1900,3,1)-DATE(1900,2,28)'; tags = @('compat-bug'); note = 'two days apart because of the phantom' }
            )
        }

        @{
            name = 'YEAR'
            cases = @(
                @{ formula = '=YEAR(45292)';   tags = @('basic') }
                @{ formula = '=YEAR(1)';       tags = @('boundary') }
                @{ formula = '=YEAR(0)';       tags = @('boundary'); note = 'serial 0 is not a real date' }
                @{ formula = '=YEAR(A2)';      tags = @('compat-bug'); note = 'serial 59' }
                @{ formula = '=YEAR(A3)';      tags = @('compat-bug'); note = 'serial 60, the phantom day' }
                @{ formula = '=YEAR(A4)';      tags = @('compat-bug'); note = 'serial 61' }
                @{ formula = '=YEAR(C2)';      tags = @('error-input', 'boundary'); note = 'negative serial' }
                @{ formula = '=YEAR(B4)';      tags = @('boundary'); note = 'the last representable serial' }
                @{ formula = '=YEAR(2958466)'; tags = @('boundary', 'overflow') }
                @{ formula = '=YEAR(45292.75)'; tags = @('boundary'); note = 'the time part is discarded' }
                @{ formula = '=YEAR(A1)';      tags = @('blank') }
                @{ formula = '=YEAR("2024-03-15")'; tags = @('coercion'); note = 'a date string coerced' }
                @{ formula = '=YEAR(B1)';      tags = @('coercion'); note = 'a cell holding an ISO date as text' }
                @{ formula = '=YEAR(B3)';      tags = @('error-input') }
            )
        }

        @{
            name = 'MONTH'
            cases = @(
                @{ formula = '=MONTH(45292)';  tags = @('basic') }
                @{ formula = '=MONTH(1)';      tags = @('boundary') }
                @{ formula = '=MONTH(0)';      tags = @('boundary') }
                @{ formula = '=MONTH(A2)';     tags = @('compat-bug') }
                @{ formula = '=MONTH(A3)';     tags = @('compat-bug'); note = 'the phantom day is in February' }
                @{ formula = '=MONTH(A4)';     tags = @('compat-bug'); note = 'March' }
                @{ formula = '=MONTH(C2)';     tags = @('error-input') }
                @{ formula = '=MONTH(B4)';     tags = @('boundary') }
                @{ formula = '=MONTH(A1)';     tags = @('blank') }
                @{ formula = '=MONTH(B1)';     tags = @('coercion') }
                @{ formula = '=MONTH(B3)';     tags = @('error-input') }
            )
        }

        @{
            name = 'DAY'
            cases = @(
                @{ formula = '=DAY(45292)';   tags = @('basic') }
                @{ formula = '=DAY(1)';       tags = @('boundary') }
                @{ formula = '=DAY(0)';       tags = @('boundary'); note = 'day 0 of January 1900' }
                @{ formula = '=DAY(A2)';      tags = @('compat-bug'); note = '28 February' }
                @{ formula = '=DAY(A3)';      tags = @('compat-bug'); note = '29 February 1900 -- the whole point' }
                @{ formula = '=DAY(A4)';      tags = @('compat-bug'); note = '1 March' }
                @{ formula = '=DAY(C2)';      tags = @('error-input') }
                @{ formula = '=DAY(B4)';      tags = @('boundary') }
                @{ formula = '=DAY(45292.99)'; tags = @('boundary') }
                @{ formula = '=DAY(A1)';      tags = @('blank') }
                @{ formula = '=DAY(B1)';      tags = @('coercion') }
                @{ formula = '=DAY(B3)';      tags = @('error-input') }
            )
        }

        @{
            name = 'WEEKDAY'
            doc  = 'Serial 1 is a Sunday in Excel-s calendar. The phantom day shifts the weekday of everything before 1 March 1900 relative to a real calendar, and the return-type argument has seven documented spellings.'
            cases = @(
                @{ formula = '=WEEKDAY(1)';        tags = @('boundary'); note = 'serial 1: Sunday, return type 1' }
                @{ formula = '=WEEKDAY(2)';        tags = @('boundary') }
                @{ formula = '=WEEKDAY(A2)';       tags = @('compat-bug'); note = 'serial 59' }
                @{ formula = '=WEEKDAY(A3)';       tags = @('compat-bug'); note = 'the phantom day has a weekday too' }
                @{ formula = '=WEEKDAY(A4)';       tags = @('compat-bug'); note = 'serial 61' }
                @{ formula = '=WEEKDAY(45292)';    tags = @('basic'); note = '1 January 2024 was a Monday' }
                @{ formula = '=WEEKDAY(45292,1)';  tags = @('basic') }
                @{ formula = '=WEEKDAY(45292,2)';  tags = @('basic'); note = 'Monday is 1' }
                @{ formula = '=WEEKDAY(45292,3)';  tags = @('basic'); note = 'Monday is 0' }
                @{ formula = '=WEEKDAY(45292,11)'; tags = @('basic') }
                @{ formula = '=WEEKDAY(45292,12)'; tags = @('basic') }
                @{ formula = '=WEEKDAY(45292,13)'; tags = @('basic') }
                @{ formula = '=WEEKDAY(45292,14)'; tags = @('basic') }
                @{ formula = '=WEEKDAY(45292,15)'; tags = @('basic') }
                @{ formula = '=WEEKDAY(45292,16)'; tags = @('basic') }
                @{ formula = '=WEEKDAY(45292,17)'; tags = @('basic') }
                @{ formula = '=WEEKDAY(45292,4)';  tags = @('error-input', 'boundary'); note = 'not a valid return type' }
                @{ formula = '=WEEKDAY(45292,0)';  tags = @('error-input') }
                @{ formula = '=WEEKDAY(0)';        tags = @('boundary') }
                @{ formula = '=WEEKDAY(C2)';       tags = @('error-input') }
                @{ formula = '=WEEKDAY(A1)';       tags = @('blank') }
                @{ formula = '=WEEKDAY(B3)';       tags = @('error-input') }
            )
        }

        @{
            name = 'TODAY'
            doc  = 'Volatile, so no fixed value can be a vector. These cases pin the structural invariants a clock cannot change: integrality, the relationship to NOW, and self-consistency within one evaluation.'
            cases = @(
                @{ formula = '=ISNUMBER(TODAY())';        tags = @('volatile', 'structural') }
                @{ formula = '=TODAY()=INT(TODAY())';     tags = @('volatile', 'structural'); note = 'no time component' }
                @{ formula = '=TODAY()-TODAY()';          tags = @('volatile', 'structural'); note = 'stable within one evaluation' }
                @{ formula = '=TODAY()>DATE(2020,1,1)';   tags = @('volatile', 'structural') }
                @{ formula = '=TODAY()<DATE(2100,1,1)';   tags = @('volatile', 'structural') }
                @{ formula = '=YEAR(TODAY())>=2024';      tags = @('volatile', 'structural') }
                @{ formula = '=TODAY()=INT(NOW())';       tags = @('volatile', 'structural'); note = 'TODAY is NOW truncated' }
            )
        }

        @{
            name = 'NOW'
            doc  = 'Volatile. Structural invariants only, for the same reason as TODAY.'
            cases = @(
                @{ formula = '=ISNUMBER(NOW())';       tags = @('volatile', 'structural') }
                @{ formula = '=NOW()>=TODAY()';        tags = @('volatile', 'structural') }
                @{ formula = '=NOW()<TODAY()+1';       tags = @('volatile', 'structural') }
                @{ formula = '=NOW()-NOW()';           tags = @('volatile', 'structural') }
                @{ formula = '=INT(NOW())=TODAY()';    tags = @('volatile', 'structural') }
                @{ formula = '=NOW()-INT(NOW())<1';    tags = @('volatile', 'structural'); note = 'the fractional part is a time of day' }
                @{ formula = '=NOW()-INT(NOW())>=0';   tags = @('volatile', 'structural') }
            )
        }
    )
}
