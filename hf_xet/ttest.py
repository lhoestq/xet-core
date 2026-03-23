import hf_xet
from huggingface_hub import HfApi
from huggingface_hub.utils import refresh_xet_connection_info

bucket_id = "lhoestq/b"
remote_path = "hey.bin"

api = HfApi()

metadata = api.get_bucket_file_metadata(bucket_id, remote_path)
headers = api._build_hf_headers()
connection_info = refresh_xet_connection_info(file_data=metadata.xet_file_data, headers=headers)


def token_refresher() -> tuple[str, int]:
    connection_info = refresh_xet_connection_info(file_data=metadata.xet_file_data, headers=headers)
    if connection_info is None:
        raise ValueError("Failed to refresh token using xet metadata.")
    return connection_info.access_token, connection_info.expiration_unix_epoch

reconstruction_summary = hf_xet.dry_download_files(
    [hf_xet.PyXetFileInfo(hash=metadata.xet_file_data.file_hash, file_size=metadata.size)],
    endpoint=connection_info.endpoint,
    token_info=(connection_info.access_token, connection_info.expiration_unix_epoch),
    token_refresher=token_refresher
)[0]
print(f"{reconstruction_summary.file_terms}")
