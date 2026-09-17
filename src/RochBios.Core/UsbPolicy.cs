namespace RochBios.Core;

public sealed record UsbDrive(int DiskNumber, string UniqueId, string Root, string Label, string Bus,
    string PartitionStyle, string FileSystem, int PartitionCount, bool IsBoot, bool IsSystem,
    long Size, long Free, long Offset, string VolumeId)
{
    public string Display => $"{Root}  {Label}  •  {Size / 1073741824.0:0.#} GB  •  {FileSystem} / {PartitionStyle}";
    public override string ToString() => Display;
    public string Identity => $"{DiskNumber}|{UniqueId}|{Root}|{Offset}|{VolumeId}";
    public string? Rejection => GetRejection(true);
    string? GetRejection(bool requireSpace) => Bus != "USB" ? "Only USB disks are supported."
        : IsBoot || IsSystem ? "System or boot disks cannot be used."
        : string.IsNullOrWhiteSpace(UniqueId) || string.IsNullOrWhiteSpace(VolumeId) ? "Drive identity could not be verified."
        : Root.Length != 3 || !char.IsAsciiLetter(Root[0]) || Root[1..] != ":\\" ? "A drive letter is required."
        : PartitionStyle != "MBR" || FileSystem != "FAT32" || PartitionCount != 1 ? "Use a FAT32 USB drive with MBR and one partition."
        : requireSpace && Free < 33558528 + 1048576 ? "At least 33 MB free is required." : null;
    public void RequireSameEligible(UsbDrive fresh, bool requireSpace = true)
    {
        if (fresh.Identity != Identity) throw new IOException("The selected drive changed. Refresh and select it again.");
        if (fresh.GetRejection(requireSpace) is string reason) throw new IOException(reason);
    }
}
