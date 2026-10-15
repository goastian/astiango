#!/usr/bin/env python3
"""Build and package the persistent native search reader with separate symbols and provenance."""
import argparse, hashlib, json, os, platform, shutil, subprocess, time, zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def sha(path):
 digest=hashlib.sha256()
 with path.open('rb') as source:
  for block in iter(lambda:source.read(1024*1024),b''):digest.update(block)
 return digest.hexdigest()
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--package-only',action='store_true');parser.add_argument('--verify-rebuild',action='store_true');args=parser.parse_args()
 digest=hashlib.sha256()
 paths=[ROOT/'Cargo.lock',ROOT/'Cargo.toml',ROOT/'rust-toolchain.toml',Path(__file__).resolve()]
 paths += sorted(path for path in (ROOT/'crates').rglob('*') if path.is_file() and (path.suffix=='.rs' or path.name=='Cargo.toml'))
 for path in paths:
  digest.update(str(path.relative_to(ROOT)).encode()+b'\0'+bytes.fromhex(sha(path)))
 build_id=digest.hexdigest();rust=subprocess.check_output(['rustc','--version'],text=True).strip()
 assert rust.split()[1]=='1.90.0',f'Pinned toolchain required: {rust}'
 lock=sha(ROOT/'Cargo.lock');commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
 environment={**os.environ,'CFLAGS':'-std=gnu17','CARGO_BUILD_JOBS':'2','SEARCH_BUILD_ID':build_id,'SEARCH_GIT_COMMIT':commit,'SEARCH_RUST_VERSION':rust,'SEARCH_LOCK_SHA256':lock}
 archive=ROOT/'data/build-assets/swagger-ui-v5.17.14.zip'
 if not archive.is_file():
  candidates=sorted((ROOT/'target').glob('*/build/utoipa-swagger-ui-*/out/v5.17.14.zip'))
  original=next((path for path in candidates if zipfile.is_zipfile(path)),None)
  if original:
   archive.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(original,archive)
 if archive.is_file():environment['SWAGGER_UI_DOWNLOAD_URL']=archive.as_uri()
 else:archive=None
 command=['cargo','build','--locked','--offline','--no-default-features','--release','-p','astiango','--bin','astiango-search-worker','--config','profile.release.package.astiango.opt-level=1','--config','profile.release.package.astiango.debug=0']
 start=time.monotonic()
 if not args.package_only:subprocess.run(command,cwd=ROOT,env=environment,check=True)
 binary=ROOT/'target/release/astiango-search-worker'
 info=json.loads(subprocess.check_output([str(binary),'--build-info'],text=True))
 assert info['build_id']==build_id,'Artifact does not match source fingerprint; rebuild required'
 directory=ROOT/'data/artifacts/search'/build_id;directory.mkdir(parents=True,exist_ok=True)
 packaged=directory/'astiango-search-worker';symbols=directory/'astiango-search-worker.debug'
 def package():
  scratch=directory/f'.package-{os.getpid()}';scratch.mkdir(exist_ok=True)
  candidate=scratch/packaged.name;debug=scratch/symbols.name
  shutil.copy2(binary,candidate)
  subprocess.run(['objcopy','--only-keep-debug',str(binary),str(debug)],check=True)
  subprocess.run(['strip','--strip-debug',str(candidate)],check=True)
  subprocess.run(['objcopy',f'--add-gnu-debuglink={debug.name}',candidate.name],cwd=scratch,check=True)
  fingerprint=sha(candidate)
  if packaged.exists():
   assert sha(packaged)==fingerprint,'Same build ID produced different bytes; existing artifact retained'
   candidate.unlink()
  else:candidate.replace(packaged)
  if symbols.exists():debug.unlink()
  else:debug.replace(symbols)
  scratch.rmdir();return fingerprint
 first=package()
 reproducible=None
 if args.verify_rebuild:
  os.utime(ROOT/"crates/core/src/bin/search_worker.rs",None)  # Invalidate the binary fingerprint, not just Cargo's output copy.
  binary.unlink();subprocess.run(command,cwd=ROOT,env=environment,check=True)
  reproducible=first==package()
  assert reproducible,'Second link differs; artifact is not reproducible on this runner'
 dependencies=subprocess.check_output(['cargo','tree','--locked','--offline','--no-default-features','-p','astiango','--prefix','none'],cwd=ROOT,text=True)
 (directory/'dependencies.txt').write_text(dependencies)
 report={'version':1,**info,'binary':str(packaged.relative_to(ROOT)),'sha256':sha(packaged),'unstripped_bytes':binary.stat().st_size,'binary_bytes':packaged.stat().st_size,'symbols_bytes':symbols.stat().st_size,'elapsed_seconds':time.monotonic()-start,'reproducible_relink':reproducible,'architecture':platform.machine(),'compiler':rust,'lock_sha256':lock,'source_files':len(paths),'standalone_evaluation':'dedicated CLI; native persistent LocalSearcher; private bounded worker protocol; shared core dependencies retained','build_command':command,'swagger_archive_sha256':sha(archive) if archive else None}
 (directory/'artifact.json').write_text(json.dumps(report,indent=2)+'\n')
 (ROOT/'data/artifacts/search/latest.json').write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps(report))
if __name__=='__main__':main()
