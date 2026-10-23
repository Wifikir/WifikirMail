-- Add worker tracking fields for job lease management
ALTER TABLE email_jobs
ADD COLUMN worker_id VARCHAR(255),
ADD COLUMN locked_until TIMESTAMPTZ;

-- Index for finding expired leases
CREATE INDEX idx_email_jobs_locked_until ON email_jobs(locked_until) WHERE locked_until IS NOT NULL;
