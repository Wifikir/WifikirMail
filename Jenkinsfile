// Jenkins: Windows agent -> SSH/rsync -> remote Ubuntu (same shape as jenkinsfile_BE).
// Point the job at this file or copy it to C:\Projects\jenkinsfile_Mail.
pipeline {
    agent { label 'BackendApi' }

    options { skipDefaultCheckout() }

    environment {
        SRC_DIR     = 'C:\\Projects\\WifikirMail'
        REMOTE_HOST = 'ubuntu@146.235.206.210'
        REMOTE_DIR  = '/opt/wifikir-mail'
        REMOTE_ENV  = '/etc/wifikir-mail/wifikir-mail.env'
        APP_USER    = 'wifikirmail'
        SERVICE     = 'wifikir-mail'
        HOME        = 'C:/Jenkins_Agents/BE'
        SSH_EXE     = 'C:/Projects/cygwin64/ssh.exe'
        RSYNC_EXE   = 'C:/Projects/cygwin64/rsync.exe'
        SSH_OPTS    = '-i C:/Projects/Oracle-Ubuntu/ssh-key-2025-11-28.key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o GlobalKnownHostsFile=/dev/null'
        HEALTH_URL  = 'http://127.0.0.1:8080/health'
        READY_URL   = 'http://127.0.0.1:8080/ready'
    }

    stages {
        stage('1. Sync source to remote') {
            steps {
                script {
                    bat """
                    ${env.SSH_EXE} ${env.SSH_OPTS} ${env.REMOTE_HOST} "sudo systemctl stop ${env.SERVICE} || true; sudo mkdir -p ${env.REMOTE_DIR}/src ${env.REMOTE_DIR}/bin; sudo chown -R ubuntu:ubuntu ${env.REMOTE_DIR}"
                    """
                    def srcCyg = env.SRC_DIR.replace('C:\\', '/cygdrive/c/').replace('\\', '/')
                    def rsh = "${env.RSYNC_EXE} -avz --checksum --delete --no-perms --no-owner --no-group --omit-dir-times -e \"${env.SSH_EXE} ${env.SSH_OPTS}\""
                    bat """
                    ${rsh} --exclude target/ --exclude .git/ ${srcCyg}/ ${env.REMOTE_HOST}:${env.REMOTE_DIR}/src/
                    """
                }
            }
        }

        stage('2. Test + build on remote') {
            steps {
                bat """
                ${env.SSH_EXE} ${env.SSH_OPTS} ${env.REMOTE_HOST} "set -e; cd ${env.REMOTE_DIR}/src; export PATH=\$HOME/.cargo/bin:\$PATH; rustc --version; cargo clippy -- -D warnings; cargo test; cargo build --release; install -m 755 target/release/wifikir_mail ${env.REMOTE_DIR}/bin/wifikir_mail; install -m 755 target/release/wifikir_mail_cli ${env.REMOTE_DIR}/bin/wifikir_mail_cli; sudo chown -R ${env.APP_USER}:${env.APP_USER} ${env.REMOTE_DIR}/bin"
                """
            }
        }

        stage('3. Restart + health') {
            steps {
                bat """
                ${env.SSH_EXE} ${env.SSH_OPTS} ${env.REMOTE_HOST} "set -e; test -f ${env.REMOTE_ENV}; sudo systemctl daemon-reload; sudo systemctl restart ${env.SERVICE}; sudo systemctl is-active ${env.SERVICE}; sleep 2; curl -fsS ${env.HEALTH_URL}; echo; curl -fsS ${env.READY_URL}; echo"
                """
            }
        }
    }

    post {
        failure {
            bat """
            ${env.SSH_EXE} ${env.SSH_OPTS} ${env.REMOTE_HOST} "sudo systemctl status ${env.SERVICE} --no-pager -l || true; sudo journalctl -u ${env.SERVICE} -n 80 --no-pager || true"
            """
        }
    }
}
