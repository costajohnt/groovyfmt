retry(times: 3, delay: 100) {
execute(
url: endpoint,
headers: [
Accept: 'application/json'
]
) { resp ->
assert resp.status == 200
}
}
