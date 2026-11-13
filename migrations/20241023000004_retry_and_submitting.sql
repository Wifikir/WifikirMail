-- Retry scheduling and ambiguous handoff support
ALTER TABLE email_jobs
ADD COLUMN next_attempt_at TIMESTAMPTZ;

-- Jobs ready for retry
CREATE INDEX idx_email_jobs_next_attempt
ON email_jobs(next_attempt_at)
WHERE status = 'Pending' AND next_attempt_at IS NOT NULL;

-- Replace partial unique index with a proper unique constraint so
-- ON CONFLICT (api_key_id, idempotency_key) works atomically.
-- Existing rows without an idempotency key get a synthetic value first.
UPDATE email_jobs
SET idempotency_key = 'legacy-' || id::text
WHERE idempotency_key IS NULL;

ALTER TABLE email_jobs
ALTER COLUMN idempotency_key SET NOT NULL;

DROP INDEX IF EXISTS unique_api_key_idempotency;

CREATE UNIQUE INDEX unique_api_key_idempotency
ON email_jobs(api_key_id, idempotency_key);
