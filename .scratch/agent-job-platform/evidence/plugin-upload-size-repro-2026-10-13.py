import urllib.request,urllib.error,json
urls=['http://192.168.1.157:5173/api/admin/plugins/install','http://192.168.1.157:8000/api/admin/plugins/install']
for url in urls:
    for size in (32,1024*1024):
        boundary='plugin-size-repro'
        body=(f'--{boundary}\r\nContent-Disposition: form-data; name="package"; filename="bad.zip"\r\nContent-Type: application/zip\r\n\r\n'.encode()+b'x'*size+f'\r\n--{boundary}--\r\n'.encode())
        request=urllib.request.Request(url,data=body,method='POST',headers={'Content-Type':f'multipart/form-data; boundary={boundary}'})
        try:
            response=urllib.request.urlopen(request,timeout=10)
        except urllib.error.HTTPError as error:
            response=error
        code=json.loads(response.read()).get('error',{}).get('code')
        print(url,'FILE_BYTES:',size,'HTTP:',response.status,'CODE:',code)
        if size==32:
            assert response.status==401, 'unauthenticated upload must be rejected'
        else:
            assert response.status==401, 'valid-size upload must reach authentication'
