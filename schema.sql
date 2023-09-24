
-- numbers are all unsigned and should be converted to that by a direct cast without changing any bits

CREATE TABLE IF NOT EXISTS ATTEMPTS (
    person BIGINT,
    day BIGINT,
    numberofguess int,
    codemode boolean
);

CREATE TABLE IF NOT EXISTS MEMBERSHIP (
    server BIGINT,
    person BIGINT
);


