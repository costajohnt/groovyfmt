node('linux') {
stage('Checkout') {
checkout scm
}
stage('Test') {
try {
sh 'make test'
} catch (err) {
currentBuild.result = 'FAILURE'
throw err
} finally {
archiveArtifacts artifacts: 'out/*.log', allowEmptyArchive: true
}
}
}
